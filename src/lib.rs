//! audioscan core library: decode an audio file once and report format, EBU
//! R128 loudness, and silence windows.
//!
//! The CLI (`src/main.rs`) is a thin wrapper over [`analyze_path`]. Library
//! callers get a typed [`ScanError`] and a serializable [`Analysis`].
//!
//! # Examples
//!
//! ```no_run
//! use audioscan::{ScanConfig, analyze_path};
//!
//! let analysis = analyze_path("take.wav", &ScanConfig::default())?;
//! println!("integrated loudness: {:?} LUFS", analysis.integrated_lufs);
//! for [start, end] in &analysis.silences {
//!     println!("silence {start}..{end}s");
//! }
//! # Ok::<(), audioscan::ScanError>(())
//! ```
#![deny(missing_docs)]

use std::fs::File;
use std::path::Path;
use std::time::Instant;

use ebur128::{EbuR128, Mode};
use serde::{Deserialize, Serialize};
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

/// A decode that recovers at least this percentage of the container's declared
/// frame count is treated as complete. The 1% slack absorbs containers that
/// over-declare `n_frames` (encoder padding, VBR estimates) without emitting a
/// false `"truncated"` warning.
const MIN_DECODED_PERCENT: u64 = 99;

/// Analysis configuration, separate from CLI and output-format concerns so the
/// library has a clean input type.
#[derive(Debug, Clone, Copy)]
pub struct ScanConfig {
    /// Silence threshold in RMS dBFS, compared against each ~30 ms window's
    /// root-mean-square level (default -30.0).
    pub threshold_db: f64,
    /// Shortest silence to report, in seconds (default 5.0).
    pub min_gap_sec: f64,
    /// Return an `Err` instead of a `"partial"` result when the decode is
    /// incomplete (corrupt packets, truncation, or an early stream end).
    pub strict: bool,
    /// Optional per-file soft decode deadline in seconds. Checked cooperatively
    /// between decoded packets, so a slow or wedged decode stops at the limit
    /// rather than running unbounded. `None` (the default) means no timeout.
    pub max_decode_secs: Option<f64>,
}

impl Default for ScanConfig {
    fn default() -> Self {
        Self {
            threshold_db: -30.0,
            min_gap_sec: 5.0,
            strict: false,
            max_decode_secs: None,
        }
    }
}

impl ScanConfig {
    /// Reject non-finite or out-of-range values before they reach the DSP or
    /// the JSON contract.
    ///
    /// # Errors
    /// Returns [`ScanError::Config`] when `threshold_db` is not finite,
    /// `min_gap_sec` is negative or not finite, or `max_decode_secs` is `Some`
    /// but not a finite positive number.
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
        if let Some(secs) = self.max_decode_secs
            && (!secs.is_finite() || secs <= 0.0)
        {
            return Err(ScanError::Config(
                "timeout must be a finite number > 0".into(),
            ));
        }
        Ok(())
    }
}

/// Typed errors a library caller can match on.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ScanError {
    /// The input file could not be opened.
    #[error("could not open {path}: {source}")]
    Open {
        /// The path that failed to open.
        path: String,
        /// The underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// The container format could not be determined.
    #[error("could not determine audio format: {0}")]
    Format(String),
    /// The file has no default (decodable) audio track.
    #[error("file has no decodable audio track")]
    NoTrack,
    /// No decoder is available for the track's codec.
    #[error("no decoder available for this codec")]
    NoDecoder,
    /// The stream is missing required info (e.g. sample rate or channel layout).
    #[error("stream is missing its {0}")]
    MissingStreamInfo(&'static str),
    /// Decoding failed, or the decode was incomplete under `strict`.
    #[error("decode failed: {0}")]
    Decode(String),
    /// The configuration was invalid (see [`ScanConfig::validate`]).
    #[error("invalid config: {0}")]
    Config(String),
}

/// Decode outcome for an [`Analysis`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// A clean decode.
    Ok,
    /// The decode completed but with diagnostics (see [`Analysis::warnings`]).
    Partial,
}

/// One analysis result. Field names are the JSON keys consumers read.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Analysis {
    /// JSON schema version (see [`SCHEMA_VERSION`]).
    pub schema_version: u32,
    /// The analysed file's path, as given.
    pub path: String,
    /// Container / extension label (e.g. `"wav"`).
    pub container: String,
    /// Codec short name (e.g. `"pcm_s16le"`, `"flac"`).
    pub codec: String,
    /// Sample rate in Hz.
    pub sample_rate: u32,
    /// Channel count.
    pub channels: u32,
    /// Bits per sample, when the codec reports it.
    pub bits_per_sample: Option<u32>,
    /// Decoded duration in seconds.
    pub duration_sec: f64,
    /// Integrated loudness (LUFS); `null` when too short or quiet to measure.
    pub integrated_lufs: Option<f64>,
    /// Loudness range (LU); `null` together with `integrated_lufs`.
    pub loudness_range_lu: Option<f64>,
    /// Maximum true peak across channels (dBTP); `null` on digital silence.
    pub true_peak_dbtp: Option<f64>,
    /// The silence threshold used, in RMS dBFS.
    pub silence_threshold_db: f64,
    /// The minimum silence gap used, in seconds.
    pub silence_min_gap_sec: f64,
    /// Silence windows as `[start_sec, end_sec]`.
    pub silences: Vec<[f64; 2]>,
    /// [`Status::Ok`] for a clean decode, [`Status::Partial`] if anything in
    /// `warnings` fired.
    pub status: Status,
    /// Count of corrupt packets skipped during decode.
    pub skipped_packets: u32,
    /// Human-readable diagnostics (truncation, skipped packets, early end,
    /// mid-file layout change). Empty on a clean decode.
    pub warnings: Vec<String>,
}

/// Streaming silence detector. Feed it per-frame mean power (mean of squared
/// samples across channels) and it emits `[start, end]` windows that stayed
/// below the threshold for at least `min_gap` seconds. O(1) state.
///
/// Per-frame power across all channels (not a mono average) means anti-phase
/// stereo is not misread as silence.
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
        // ~30 ms analysis window, the granularity ffmpeg's silencedetect uses.
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
///
/// # Errors
/// Returns [`ScanError::Config`] for an invalid [`ScanConfig`],
/// [`ScanError::Open`] if the file cannot be opened, [`ScanError::Format`] if
/// the container is unrecognised, [`ScanError::NoTrack`] /
/// [`ScanError::NoDecoder`] / [`ScanError::MissingStreamInfo`] for an
/// undecodable stream, and [`ScanError::Decode`] on a decode failure or, under
/// [`ScanConfig::strict`], any incomplete decode.
pub fn analyze_path(path: impl AsRef<Path>, config: &ScanConfig) -> Result<Analysis, ScanError> {
    config.validate()?;
    let path = path.as_ref();

    let file = File::open(path).map_err(|e| ScanError::Open {
        path: path.display().to_string(),
        source: e,
    })?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    let container = path
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
            &FormatOptions {
                // Honour encoder delay/padding so MP3/AAC durations and silence
                // offsets track the true media timeline.
                enable_gapless: true,
                ..Default::default()
            },
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
    let declared_frames = params.n_frames;
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
    let mut early_end = false;
    let mut timed_out = false;
    let mut layout_changed: Option<(u32, u32)> = None;
    let ch = channels.max(1) as usize;
    let decode_started = Instant::now();

    loop {
        // Cooperative soft deadline: a slow or wedged file stops at the limit
        // between packets rather than decoding unbounded.
        if let Some(limit) = config.max_decode_secs
            && decode_started.elapsed().as_secs_f64() > limit
        {
            timed_out = true;
            break;
        }
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(SymError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(SymError::ResetRequired) => {
                early_end = true;
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
                early_end = true;
                break;
            }
            Err(e) => return Err(ScanError::Decode(format!("decoding: {e}"))),
        };

        let spec = *decoded.spec();
        // The loudness + silence models were built for the header layout. A real
        // mid-stream channel/rate change can't be fed to them correctly, and a
        // wider packet would overflow the sample buffer's capacity assert, so
        // flag it and stop rather than panic or mix layouts.
        if spec.channels.count() as u32 != channels || spec.rate != sample_rate {
            layout_changed = Some((spec.channels.count() as u32, spec.rate));
            break;
        }

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
    // Loudness range follows integrated loudness: ebur128 returns Ok(0.0) for an
    // empty gating history, which would be a misleading 0.0 against a null
    // integrated value, so gate it to None whenever integrated is None.
    let loudness_range_lu = integrated_lufs.and_then(|_| {
        ebu.loudness_range()
            .ok()
            .filter(|v| v.is_finite())
            .map(round2)
    });
    let true_peak_dbtp = max_true_peak(&ebu, channels);

    let mut warnings: Vec<String> = Vec::new();
    if skipped_packets > 0 {
        warnings.push(format!("skipped {skipped_packets} corrupt packet(s)"));
    }
    if early_end {
        warnings.push("decode ended early on a stream error".to_string());
    }
    if timed_out && let Some(limit) = config.max_decode_secs {
        warnings.push(format!("decode exceeded timeout of {limit}s"));
    }
    if let Some((a, r)) = layout_changed {
        warnings.push(format!(
            "stream changed layout mid-file: {channels}ch/{sample_rate}Hz -> {a}ch/{r}Hz"
        ));
    }
    // Truncation: decoded materially fewer frames than the container declared.
    if let Some(declared) = declared_frames
        && declared > 0
        && total_frames < declared.saturating_mul(MIN_DECODED_PERCENT) / 100
    {
        let declared_sec = round3(declared as f64 / sample_rate as f64);
        warnings.push(format!(
            "truncated: decoded {duration_sec}s of {declared_sec}s declared"
        ));
    }
    let status = if warnings.is_empty() {
        Status::Ok
    } else {
        Status::Partial
    };

    if config.strict && !warnings.is_empty() {
        return Err(ScanError::Decode(format!(
            "incomplete decode (strict): {}",
            warnings.join("; ")
        )));
    }

    Ok(Analysis {
        schema_version: SCHEMA_VERSION,
        path: path.display().to_string(),
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
        warnings,
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
    // peak == 0.0 means digital silence (no inter-sample peak); report None by
    // design rather than -inf or 0.0 dBTP.
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
