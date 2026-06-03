//! audioscan — decode an audio file once and report its specs, EBU R128 loudness,
//! and silence windows as JSON.
//!
//! Why it exists: endenza-bootleg currently shells out to `ffmpeg` two or three
//! times per file (loudness, silence, duration), fully decoding each time and
//! scraping stderr text. This does it in a single pass over the decoded samples
//! with the real `ebur128` library, and emits structured JSON instead of text to
//! regex. Same numbers, fewer decodes, no parsing fragility, trivially parallel.
//!
//!     audioscan [--compact] [--threshold <dB>] [--min-gap <s>] <file>

use std::fs::File;
use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};
use ebur128::{EbuR128, Mode};
use serde::Serialize;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

/// One analysis result. Field names are the JSON keys Bootleg will read.
#[derive(Serialize)]
struct Analysis {
    path: String,
    container: String,
    codec: String,
    sample_rate: u32,
    channels: u32,
    bits_per_sample: Option<u32>,
    duration_sec: f64,
    /// Integrated loudness (LUFS). `None` for a file too quiet/short to measure.
    integrated_lufs: Option<f64>,
    /// Loudness range (LU).
    loudness_range_lu: Option<f64>,
    /// Maximum true peak across channels (dBTP).
    true_peak_dbtp: Option<f64>,
    silence_threshold_db: f64,
    silence_min_gap_sec: f64,
    /// Silence windows as `[start_sec, end_sec]`, same convention Bootleg's
    /// segmentation consumes.
    silences: Vec<[f64; 2]>,
}

struct Config {
    path: String,
    threshold_db: f64,
    min_gap_sec: f64,
    pretty: bool,
}

fn parse_args() -> Result<Config> {
    // Bootleg's tuned defaults (see silencedetect.py): -30 dB over a 5 s gap.
    let mut path: Option<String> = None;
    let mut threshold_db = -30.0_f64;
    let mut min_gap_sec = 5.0_f64;
    let mut pretty = true;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--compact" => pretty = false,
            "--pretty" => pretty = true,
            "--threshold" => {
                threshold_db = args
                    .next()
                    .ok_or_else(|| anyhow!("--threshold needs a value (dB)"))?
                    .parse()
                    .context("--threshold must be a number")?;
            }
            "--min-gap" => {
                min_gap_sec = args
                    .next()
                    .ok_or_else(|| anyhow!("--min-gap needs a value (seconds)"))?
                    .parse()
                    .context("--min-gap must be a number")?;
            }
            "-h" | "--help" => {
                println!("usage: audioscan [--compact] [--threshold <dB>] [--min-gap <s>] <file>");
                std::process::exit(0);
            }
            flag if flag.starts_with('-') => bail!("unknown flag: {flag}"),
            file => {
                if path.is_some() {
                    bail!("only one input file is supported");
                }
                path = Some(file.to_string());
            }
        }
    }

    Ok(Config {
        path: path.ok_or_else(|| anyhow!("no input file (usage: audioscan <file>)"))?,
        threshold_db,
        min_gap_sec,
        pretty,
    })
}

/// A small streaming silence detector: feed it mono samples, it emits the
/// `[start, end]` windows where the signal stayed below the threshold for at
/// least `min_gap` seconds. O(1) memory, so it scales to multi-hour tapes.
struct SilenceTracker {
    sample_rate: f64,
    threshold_db: f64,
    min_gap: f64,
    win_frames: u64,
    win_sumsq: f64,
    win_filled: u64,
    frames_seen: u64,
    silence_start: Option<f64>,
    out: Vec<[f64; 2]>,
}

impl SilenceTracker {
    fn new(sample_rate: u32, threshold_db: f64, min_gap: f64) -> Self {
        // ~30 ms analysis window, matching the granularity ffmpeg's
        // silencedetect works at.
        let win = ((sample_rate as f64) * 0.030).max(1.0) as u64;
        Self {
            sample_rate: sample_rate as f64,
            threshold_db,
            min_gap,
            win_frames: win,
            win_sumsq: 0.0,
            win_filled: 0,
            frames_seen: 0,
            silence_start: None,
            out: Vec::new(),
        }
    }

    fn push(&mut self, mono: f32) {
        self.win_sumsq += (mono as f64) * (mono as f64);
        self.win_filled += 1;
        self.frames_seen += 1;
        if self.win_filled >= self.win_frames {
            self.flush_window();
        }
    }

    fn flush_window(&mut self) {
        if self.win_filled == 0 {
            return;
        }
        let rms = (self.win_sumsq / self.win_filled as f64).sqrt();
        let db = if rms > 1e-12 {
            20.0 * rms.log10()
        } else {
            -200.0
        };
        let win_end = self.frames_seen as f64 / self.sample_rate;
        let win_start = win_end - (self.win_filled as f64 / self.sample_rate);

        if db < self.threshold_db {
            if self.silence_start.is_none() {
                self.silence_start = Some(win_start);
            }
        } else if let Some(start) = self.silence_start.take() {
            if win_start - start >= self.min_gap {
                self.out.push([round3(start), round3(win_start)]);
            }
        }
        self.win_sumsq = 0.0;
        self.win_filled = 0;
    }

    fn finish(mut self) -> Vec<[f64; 2]> {
        self.flush_window();
        if let Some(start) = self.silence_start.take() {
            let end = self.frames_seen as f64 / self.sample_rate;
            if end - start >= self.min_gap {
                self.out.push([round3(start), round3(end)]);
            }
        }
        self.out
    }
}

fn analyze(cfg: &Config) -> Result<Analysis> {
    let file = File::open(&cfg.path).with_context(|| format!("opening {}", cfg.path))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    // Give the prober the extension as a hint; it still sniffs the bytes.
    let mut hint = Hint::new();
    let container = Path::new(&cfg.path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();
    if !container.is_empty() {
        hint.with_extension(&container);
    }

    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .context("could not determine audio format")?;
    let mut format = probed.format;

    let track = format
        .default_track()
        .ok_or_else(|| anyhow!("file has no default audio track"))?;
    let track_id = track.id;
    let params = track.codec_params.clone();

    let sample_rate = params
        .sample_rate
        .ok_or_else(|| anyhow!("stream has no sample rate"))?;
    let channels = params
        .channels
        .map(|c| c.count() as u32)
        .ok_or_else(|| anyhow!("stream has no channel layout"))?;
    let bits_per_sample = params.bits_per_sample;
    let codec = symphonia::default::get_codecs()
        .get_codec(params.codec)
        .map(|d| d.short_name.to_string())
        .unwrap_or_else(|| "unknown".to_string());

    let mut decoder = symphonia::default::get_codecs()
        .make(&params, &DecoderOptions::default())
        .context("no decoder available for this codec")?;

    let mut ebu = EbuR128::new(channels, sample_rate, Mode::I | Mode::LRA | Mode::TRUE_PEAK)
        .context("initialising ebur128")?;
    let mut silence = SilenceTracker::new(sample_rate, cfg.threshold_db, cfg.min_gap_sec);

    let mut sample_buf: Option<SampleBuffer<f32>> = None;
    let mut buf_cap_frames: u64 = 0;
    let mut total_frames: u64 = 0;
    let ch = channels as usize;

    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            // End of stream is reported as an unexpected-EOF io error.
            Err(SymError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(SymError::ResetRequired) => break,
            Err(e) => return Err(e).context("reading packet"),
        };
        if packet.track_id() != track_id {
            continue;
        }

        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            // A single corrupt packet shouldn't abort a multi-hour scan.
            Err(SymError::DecodeError(_)) => continue,
            Err(SymError::IoError(_)) => break,
            Err(e) => return Err(e).context("decoding packet"),
        };

        let spec = *decoded.spec();
        let frames = decoded.frames();
        let cap = decoded.capacity() as u64;
        if sample_buf.is_none() || frames as u64 > buf_cap_frames {
            buf_cap_frames = cap;
            sample_buf = Some(SampleBuffer::<f32>::new(buf_cap_frames, spec));
        }
        let buf = sample_buf.as_mut().unwrap();
        buf.copy_interleaved_ref(decoded);
        let samples = buf.samples();

        // Feed the loudness meter the interleaved frames as-is.
        ebu.add_frames_f32(samples).context("ebur128 add_frames")?;

        // Feed the silence tracker one mono-folded sample per frame.
        for f in 0..frames {
            let mut acc = 0.0f32;
            for c in 0..ch {
                acc += samples[f * ch + c];
            }
            silence.push(acc / ch as f32);
        }
        total_frames += frames as u64;
    }

    let duration_sec = round3(total_frames as f64 / sample_rate as f64);
    let integrated_lufs = ebu
        .loudness_global()
        .ok()
        .filter(|v| v.is_finite())
        .map(round2);
    let loudness_range_lu = ebu
        .loudness_range()
        .ok()
        .filter(|v| v.is_finite())
        .map(round2);
    let true_peak_dbtp = max_true_peak(&ebu, channels);

    Ok(Analysis {
        path: cfg.path.clone(),
        container,
        codec,
        sample_rate,
        channels,
        bits_per_sample,
        duration_sec,
        integrated_lufs,
        loudness_range_lu,
        true_peak_dbtp,
        silence_threshold_db: cfg.threshold_db,
        silence_min_gap_sec: cfg.min_gap_sec,
        silences: silence.finish(),
    })
}

/// Largest true peak across all channels, in dBTP (`None` if unmeasurable).
fn max_true_peak(ebu: &EbuR128, channels: u32) -> Option<f64> {
    let mut peak = 0.0f64;
    let mut measured = false;
    for c in 0..channels {
        if let Ok(p) = ebu.true_peak(c) {
            measured = true;
            if p > peak {
                peak = p;
            }
        }
    }
    if measured && peak > 0.0 {
        Some(round2(20.0 * peak.log10()))
    } else {
        None
    }
}

fn round2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

fn round3(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}

fn main() {
    let cfg = match parse_args() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("audioscan: {e:#}");
            std::process::exit(2);
        }
    };

    match analyze(&cfg) {
        Ok(analysis) => {
            let json = if cfg.pretty {
                serde_json::to_string_pretty(&analysis)
            } else {
                serde_json::to_string(&analysis)
            };
            match json {
                Ok(s) => println!("{s}"),
                Err(e) => {
                    eprintln!("audioscan: could not serialize result: {e}");
                    std::process::exit(1);
                }
            }
        }
        Err(e) => {
            eprintln!("audioscan: {e:#}");
            std::process::exit(1);
        }
    }
}
