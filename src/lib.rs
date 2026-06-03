//! audioscan core library: decode an audio file once and report format, EBU
//! R128 loudness, and silence windows.
//!
//! The CLI (`src/main.rs`) is a thin wrapper over [`analyze_path`]. Library
//! callers get a typed [`ScanError`] and a serializable [`Analysis`]; the binary
//! maps the error to stderr plus an exit code.

use std::fs::File;
use std::path::Path;

use ebur128::{EbuR128, Mode};
use serde::Serialize;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use thiserror::Error;

/// JSON schema version. Bump only on a breaking change to the field set; new
/// fields are additive and do not bump it.
pub const SCHEMA_VERSION: u32 = 1;

/// Analysis configuration, separate from CLI and output-format concerns so the
/// library has a clean input type.
#[derive(Debug, Clone, Copy)]
pub struct ScanConfig {
    /// Silence threshold in dBFS (default -30.0).
    pub threshold_db: f64,
    /// Shortest silence to report, in seconds (default 5.0).
    pub min_gap_sec: f64,
}

impl Default for ScanConfig {
    fn default() -> Self {
        Self {
            threshold_db: -30.0,
            min_gap_sec: 5.0,
        }
    }
}

impl ScanConfig {
    /// Reject non-finite or out-of-range values before they reach the DSP or
    /// the JSON contract.
    pub fn validate(&self) -> Result<(), ScanError> {
        if !self.threshold_db.is_finite() {
            return Err(ScanError::Config(
                "threshold must be a finite number".into(),
            ));
        }
        if !self.min_gap_sec.is_finite() || self.min_gap_sec < 0.0 {
            return Err(ScanError::Config(
                "min-gap must be a finite number >= 0".into(),
            ));
        }
        Ok(())
    }
}

/// Typed errors a library caller can match on.
#[derive(Debug, Error)]
pub enum ScanError {
    #[error("could not open {path}: {source}")]
    Open {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("could not determine audio format: {0}")]
    Format(String),
    #[error("file has no decodable audio track")]
    NoTrack,
    #[error("no decoder available for this codec")]
    NoDecoder,
    #[error("stream is missing its {0}")]
    MissingStreamInfo(&'static str),
    #[error("decode failed: {0}")]
    Decode(String),
    #[error("invalid config: {0}")]
    Config(String),
}

/// One analysis result. Field names are the JSON keys consumers read; loudness
/// fields are `null` when a file is too short or quiet to measure.
#[derive(Debug, Serialize)]
pub struct Analysis {
    pub schema_version: u32,
    pub path: String,
    pub container: String,
    pub codec: String,
    pub sample_rate: u32,
    pub channels: u32,
    pub bits_per_sample: Option<u32>,
    pub duration_sec: f64,
    pub integrated_lufs: Option<f64>,
    pub loudness_range_lu: Option<f64>,
    pub true_peak_dbtp: Option<f64>,
    pub silence_threshold_db: f64,
    pub silence_min_gap_sec: f64,
    pub silences: Vec<[f64; 2]>,
    /// `"ok"` for a clean decode, `"partial"` if packets were skipped or the
    /// stream ended unexpectedly mid-decode.
    pub status: &'static str,
    /// Count of corrupt packets skipped during decode.
    pub skipped_packets: u32,
}

/// Streaming silence detector. Feed it per-frame mean power (mean of squared
/// samples across channels) and it emits `[start, end]` windows that stayed
/// below the threshold for at least `min_gap` seconds. O(1) state.
///
/// Using per-frame power *across all channels* (not a mono average) is the fix
/// for the anti-phase bug: a stereo frame `[+1.0, -1.0]` has mean power 1.0, so
/// full-scale out-of-phase audio is no longer misread as silence.
struct SilenceTracker {
    sample_rate: f64,
    threshold_db: f64,
    min_gap: f64,
    win_frames: u64,
    win_power_sum: f64,
    win_filled: u64,
    frames_seen: u64,
    silence_start: Option<f64>,
    out: Vec<[f64; 2]>,
}

impl SilenceTracker {
    fn new(sample_rate: u32, threshold_db: f64, min_gap: f64) -> Self {
        // ~30 ms analysis window, the granularity ffmpeg's silencedetect works at.
        let win = ((sample_rate as f64) * 0.030).max(1.0) as u64;
        Self {
            sample_rate: sample_rate as f64,
            threshold_db,
            min_gap,
            win_frames: win,
            win_power_sum: 0.0,
            win_filled: 0,
            frames_seen: 0,
            silence_start: None,
            out: Vec::new(),
        }
    }

    /// `frame_power` is the mean of squared samples across channels for one frame.
    fn push(&mut self, frame_power: f64) {
        self.win_power_sum += frame_power;
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
        let rms = (self.win_power_sum / self.win_filled as f64).sqrt();
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
        } else if let Some(start) = self.silence_start.take()
            && win_start - start >= self.min_gap
        {
            self.out.push([round3(start), round3(win_start)]);
        }
        self.win_power_sum = 0.0;
        self.win_filled = 0;
    }

    fn finish(mut self) -> Vec<[f64; 2]> {
        self.flush_window();
        let end = self.frames_seen as f64 / self.sample_rate;
        if let Some(start) = self.silence_start.take()
            && end - start >= self.min_gap
        {
            self.out.push([round3(start), round3(end)]);
        }
        self.out
    }
}

/// Decode `path` once and return its [`Analysis`].
pub fn analyze_path(path: &str, config: &ScanConfig) -> Result<Analysis, ScanError> {
    config.validate()?;

    let file = File::open(path).map_err(|e| ScanError::Open {
        path: path.to_string(),
        source: e,
    })?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    let container = Path::new(path)
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
        .map_err(|e| ScanError::Format(e.to_string()))?;
    let mut format = probed.format;

    let track = format.default_track().ok_or(ScanError::NoTrack)?;
    let track_id = track.id;
    let params = track.codec_params.clone();

    let sample_rate = params
        .sample_rate
        .ok_or(ScanError::MissingStreamInfo("sample rate"))?;
    let channels = params
        .channels
        .map(|c| c.count() as u32)
        .ok_or(ScanError::MissingStreamInfo("channel layout"))?;
    let bits_per_sample = params.bits_per_sample;
    let codec = symphonia::default::get_codecs()
        .get_codec(params.codec)
        .map(|d| d.short_name.to_string())
        .unwrap_or_else(|| "unknown".to_string());

    let mut decoder = symphonia::default::get_codecs()
        .make(&params, &DecoderOptions::default())
        .map_err(|_| ScanError::NoDecoder)?;

    let mut ebu = EbuR128::new(channels, sample_rate, Mode::I | Mode::LRA | Mode::TRUE_PEAK)
        .map_err(|e| ScanError::Decode(format!("ebur128 init: {e}")))?;
    let mut silence = SilenceTracker::new(sample_rate, config.threshold_db, config.min_gap_sec);

    let mut sample_buf: Option<SampleBuffer<f32>> = None;
    let mut buf_cap_frames: u64 = 0;
    let mut total_frames: u64 = 0;
    let mut skipped_packets: u32 = 0;
    let mut partial = false;
    let ch = channels.max(1) as usize;

    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(SymError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(SymError::ResetRequired) => {
                partial = true;
                break;
            }
            Err(e) => return Err(ScanError::Decode(format!("reading packet: {e}"))),
        };
        if packet.track_id() != track_id {
            continue;
        }

        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            // A single corrupt packet is skipped and counted, not fatal.
            Err(SymError::DecodeError(_)) => {
                skipped_packets += 1;
                continue;
            }
            Err(SymError::IoError(_)) => {
                partial = true;
                break;
            }
            Err(e) => return Err(ScanError::Decode(format!("decoding: {e}"))),
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

        ebu.add_frames_f32(samples)
            .map_err(|e| ScanError::Decode(format!("ebur128 add_frames: {e}")))?;

        // Iterate only whole frames actually present, so a buffer whose length
        // is not frames*channels can never index out of bounds.
        let avail_frames = samples.len() / ch;
        for f in 0..avail_frames {
            let mut sumsq = 0.0f64;
            for c in 0..ch {
                let s = samples[f * ch + c] as f64;
                sumsq += s * s;
            }
            silence.push(sumsq / ch as f64);
        }
        total_frames += avail_frames as u64;
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
    let status = if partial || skipped_packets > 0 {
        "partial"
    } else {
        "ok"
    };

    Ok(Analysis {
        schema_version: SCHEMA_VERSION,
        path: path.to_string(),
        container,
        codec,
        sample_rate,
        channels,
        bits_per_sample,
        duration_sec,
        integrated_lufs,
        loudness_range_lu,
        true_peak_dbtp,
        silence_threshold_db: config.threshold_db,
        silence_min_gap_sec: config.min_gap_sec,
        silences: silence.finish(),
        status,
        skipped_packets,
    })
}

/// Largest true peak across all channels in dBTP (`None` if unmeasurable or
/// non-finite).
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
        let dbtp = 20.0 * peak.log10();
        dbtp.is_finite().then(|| round2(dbtp))
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
