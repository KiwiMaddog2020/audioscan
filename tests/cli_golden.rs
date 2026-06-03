use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::Value;

const SAMPLE_RATE: u32 = 48_000;
static NEXT_FIXTURE_DIR: AtomicUsize = AtomicUsize::new(0);

struct FixtureDir {
    path: PathBuf,
}

impl FixtureDir {
    fn new(name: &str) -> Self {
        let unique = NEXT_FIXTURE_DIR.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "audioscan-cli-golden-{name}-{}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).expect("create fixture dir");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for FixtureDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn audioscan_output(path: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_audioscan"))
        .arg("--compact")
        .arg(path)
        .output()
        .expect("run audioscan")
}

fn audioscan_json(path: &Path) -> Value {
    let output = audioscan_output(path);
    assert_eq!(
        output.status.code(),
        Some(0),
        "expected audioscan success\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("audioscan stdout is JSON")
}

fn field<'a>(json: &'a Value, name: &str) -> &'a Value {
    json.get(name)
        .unwrap_or_else(|| panic!("missing JSON field {name}"))
}

fn number_field(json: &Value, name: &str) -> f64 {
    field(json, name)
        .as_f64()
        .unwrap_or_else(|| panic!("{name} should be a JSON number"))
}

fn array_field<'a>(json: &'a Value, name: &str) -> &'a Vec<Value> {
    field(json, name)
        .as_array()
        .unwrap_or_else(|| panic!("{name} should be a JSON array"))
}

fn assert_approx(actual: f64, expected: f64, tolerance: f64, label: &str) {
    assert!(
        (actual - expected).abs() <= tolerance,
        "{label}: expected {actual} to be within {tolerance} of {expected}"
    );
}

fn assert_finite_negative(json: &Value, name: &str) {
    let value = number_field(json, name);
    assert!(value.is_finite(), "{name} should be finite");
    assert!(value < 0.0, "{name} should be negative, got {value}");
}

fn assert_common_wav_fields(json: &Value, path: &Path, channels: u64, duration_sec: f64) {
    assert_eq!(field(json, "schema_version").as_u64(), Some(1));
    assert_eq!(field(json, "path").as_str(), Some(path.to_str().unwrap()));
    assert_eq!(field(json, "container").as_str(), Some("wav"));
    assert_eq!(field(json, "codec").as_str(), Some("pcm_s16le"));
    assert_eq!(
        field(json, "sample_rate").as_u64(),
        Some(SAMPLE_RATE as u64)
    );
    assert_eq!(field(json, "channels").as_u64(), Some(channels));
    assert_eq!(field(json, "bits_per_sample").as_u64(), Some(16));
    assert_approx(
        number_field(json, "duration_sec"),
        duration_sec,
        0.05,
        "duration_sec",
    );
    assert_approx(
        number_field(json, "silence_threshold_db"),
        -30.0,
        f64::EPSILON,
        "silence_threshold_db",
    );
    assert_approx(
        number_field(json, "silence_min_gap_sec"),
        5.0,
        f64::EPSILON,
        "silence_min_gap_sec",
    );
    assert_eq!(field(json, "status").as_str(), Some("ok"));
    assert_eq!(field(json, "skipped_packets").as_u64(), Some(0));
}

fn assert_loudness_metrics_present(json: &Value) {
    assert_finite_negative(json, "integrated_lufs");
    assert!(number_field(json, "loudness_range_lu").is_finite());
    assert_finite_negative(json, "true_peak_dbtp");
}

fn write_mono_wav(path: &Path, samples: impl IntoIterator<Item = f32>) {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: SAMPLE_RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(path, spec).expect("create mono wav");
    for sample in samples {
        writer
            .write_sample(float_to_i16(sample))
            .expect("write mono sample");
    }
    writer.finalize().expect("finalize mono wav");
}

fn write_stereo_wav(path: &Path, frames: impl IntoIterator<Item = (f32, f32)>) {
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: SAMPLE_RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(path, spec).expect("create stereo wav");
    for (left, right) in frames {
        writer
            .write_sample(float_to_i16(left))
            .expect("write left sample");
        writer
            .write_sample(float_to_i16(right))
            .expect("write right sample");
    }
    writer.finalize().expect("finalize stereo wav");
}

fn float_to_i16(sample: f32) -> i16 {
    (sample.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16
}

fn sine_sample(frame: u32, freq_hz: f32, amplitude: f32) -> f32 {
    let phase = 2.0 * std::f32::consts::PI * freq_hz * frame as f32 / SAMPLE_RATE as f32;
    amplitude * phase.sin()
}

fn silence_window(json: &Value, index: usize) -> (f64, f64) {
    let window = array_field(json, "silences")
        .get(index)
        .unwrap_or_else(|| panic!("missing silence window {index}"))
        .as_array()
        .unwrap_or_else(|| panic!("silence window {index} should be an array"));
    assert_eq!(
        window.len(),
        2,
        "silence window should contain start and end"
    );
    (
        window[0].as_f64().expect("silence start is a number"),
        window[1].as_f64().expect("silence end is a number"),
    )
}

#[test]
fn mono_tone_with_middle_silence_reports_expected_window() {
    let dir = FixtureDir::new("mono-gap");
    let path = dir.path().join("mono_gap.wav");
    let tone_frames = SAMPLE_RATE;
    let gap_frames = (SAMPLE_RATE as f32 * 6.2) as u32;

    let samples = (0..tone_frames)
        .map(|frame| sine_sample(frame, 1_000.0, 0.5))
        .chain((0..gap_frames).map(|_| 0.0))
        .chain((0..tone_frames).map(|frame| sine_sample(frame, 1_000.0, 0.5)));
    write_mono_wav(&path, samples);

    let json = audioscan_json(&path);

    assert_common_wav_fields(&json, &path, 1, 8.2);
    assert_loudness_metrics_present(&json);
    let silences = array_field(&json, "silences");
    assert_eq!(silences.len(), 1);
    let (start, end) = silence_window(&json, 0);
    assert_approx(start, 1.02, 0.1, "silence start");
    assert_approx(end, 7.2, 0.1, "silence end");
}

#[test]
fn anti_phase_stereo_does_not_report_false_silence() {
    let dir = FixtureDir::new("antiphase");
    let path = dir.path().join("antiphase.wav");
    let frames = (0..SAMPLE_RATE * 2).map(|frame| {
        let sample = sine_sample(frame, 1_000.0, 0.7);
        (sample, -sample)
    });
    write_stereo_wav(&path, frames);

    let json = audioscan_json(&path);

    // Guards the multichannel-power silence fix: anti-phase stereo must not
    // collapse to a false whole-file silence window.
    assert_common_wav_fields(&json, &path, 2, 2.0);
    assert_loudness_metrics_present(&json);
    assert!(array_field(&json, "silences").is_empty());
}

#[test]
fn pure_silence_reports_whole_file_window_and_null_loudness() {
    let dir = FixtureDir::new("silence");
    let path = dir.path().join("silence.wav");
    write_mono_wav(&path, (0..(SAMPLE_RATE as f32 * 6.25) as u32).map(|_| 0.0));

    let json = audioscan_json(&path);

    assert_common_wav_fields(&json, &path, 1, 6.25);
    assert_eq!(field(&json, "integrated_lufs"), &Value::Null);
    assert_eq!(field(&json, "loudness_range_lu"), &Value::Null);
    assert_eq!(field(&json, "true_peak_dbtp"), &Value::Null);
    let silences = array_field(&json, "silences");
    assert_eq!(silences.len(), 1);
    let (start, end) = silence_window(&json, 0);
    assert_approx(start, 0.0, 0.05, "silence start");
    assert_approx(end, 6.25, 0.05, "silence end");
}

#[test]
fn short_file_exits_successfully_and_reports_duration() {
    let dir = FixtureDir::new("short");
    let path = dir.path().join("short.wav");
    let duration = 0.35;
    let frames = (SAMPLE_RATE as f32 * duration) as u32;
    write_mono_wav(
        &path,
        (0..frames).map(|frame| sine_sample(frame, 440.0, 0.4)),
    );

    let json = audioscan_json(&path);

    assert_common_wav_fields(&json, &path, 1, duration as f64);
    assert_eq!(field(&json, "integrated_lufs"), &Value::Null);
    assert_eq!(field(&json, "loudness_range_lu"), &Value::Null);
    assert_finite_negative(&json, "true_peak_dbtp");
    assert!(array_field(&json, "silences").is_empty());
}

#[test]
fn truncated_wav_does_not_abort() {
    let dir = FixtureDir::new("truncated");
    let path = dir.path().join("truncated.wav");
    write_mono_wav(
        &path,
        (0..SAMPLE_RATE).map(|frame| sine_sample(frame, 220.0, 0.2)),
    );
    let bytes = std::fs::read(&path).expect("read complete wav");
    std::fs::write(&path, &bytes[..80]).expect("write truncated wav");

    let output = audioscan_output(&path);
    let code = output
        .status
        .code()
        .expect("audioscan terminated by signal");
    assert!(
        matches!(code, 0 | 1),
        "truncated input should exit 0 or 1, got {code}\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    if output.stdout.iter().any(|byte| !byte.is_ascii_whitespace()) {
        let json: Value = serde_json::from_slice(&output.stdout).expect("stdout JSON");
        assert!(field(&json, "status").as_str().is_some());
        assert!(field(&json, "skipped_packets").as_u64().is_some());
    }
}

#[test]
fn invalid_numeric_args_exit_with_usage_error() {
    let dir = FixtureDir::new("bad-args");
    let path = dir.path().join("valid.wav");
    write_mono_wav(
        &path,
        (0..SAMPLE_RATE / 10).map(|frame| sine_sample(frame, 440.0, 0.2)),
    );

    for args in [
        ["--threshold", "nan", path.to_str().unwrap()],
        ["--min-gap", "-5", path.to_str().unwrap()],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_audioscan"))
            .args(args)
            .output()
            .expect("run audioscan with bad args");
        assert_eq!(
            output.status.code(),
            Some(2),
            "bad args should exit 2\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            !output.stderr.is_empty(),
            "bad args should emit a non-empty stderr message"
        );
    }
}
