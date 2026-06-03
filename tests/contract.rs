use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

use hound::{SampleFormat, WavSpec, WavWriter};
use serde_json::Value;

const SAMPLE_RATE: u32 = 48_000;
static NEXT_FIXTURE_DIR: AtomicUsize = AtomicUsize::new(0);

const CONTRACT_FIELDS: [&str; 17] = [
    "schema_version",
    "path",
    "container",
    "codec",
    "sample_rate",
    "channels",
    "bits_per_sample",
    "duration_sec",
    "integrated_lufs",
    "loudness_range_lu",
    "true_peak_dbtp",
    "silence_threshold_db",
    "silence_min_gap_sec",
    "silences",
    "status",
    "skipped_packets",
    "warnings",
];

struct FixtureDir {
    path: PathBuf,
}

impl FixtureDir {
    fn new(name: &str) -> Self {
        let unique = NEXT_FIXTURE_DIR.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "audioscan-contract-{name}-{}-{unique}",
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

fn audioscan_output(path: &Path, strict: bool) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_audioscan"));
    command.arg("--compact");
    if strict {
        command.arg("--strict");
    }
    command.arg(path).output().expect("run audioscan")
}

fn audioscan_json(path: &Path, strict: bool) -> Value {
    let output = audioscan_output(path, strict);
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

fn warnings(json: &Value) -> &Vec<Value> {
    field(json, "warnings")
        .as_array()
        .expect("warnings should be a JSON array")
}

fn assert_approx(actual: f64, expected: f64, tolerance: f64, label: &str) {
    assert!(
        (actual - expected).abs() <= tolerance,
        "{label}: expected {actual} to be within {tolerance} of {expected}"
    );
}

fn assert_clean_ok(json: &Value) {
    assert_eq!(field(json, "status").as_str(), Some("ok"));
    assert!(warnings(json).is_empty(), "warnings should be []");
}

fn write_mono_wav(path: &Path, samples: impl IntoIterator<Item = f32>) {
    let spec = WavSpec {
        channels: 1,
        sample_rate: SAMPLE_RATE,
        bits_per_sample: 16,
        sample_format: SampleFormat::Int,
    };
    let mut writer = WavWriter::create(path, spec).expect("create mono wav");
    for sample in samples {
        writer
            .write_sample(float_to_i16(sample))
            .expect("write mono sample");
    }
    writer.finalize().expect("finalize mono wav");
}

fn write_clean_gap_wav(path: &Path) {
    let tone_frames = SAMPLE_RATE;
    let gap_frames = (SAMPLE_RATE as f32 * 6.2) as u32;
    let samples = (0..tone_frames)
        .map(|frame| sine_sample(frame, 1_000.0, 0.5))
        .chain((0..gap_frames).map(|_| 0.0))
        .chain((0..tone_frames).map(|frame| sine_sample(frame, 1_000.0, 0.5)));
    write_mono_wav(path, samples);
}

fn write_tone_wav(path: &Path, duration_sec: f64) {
    let frames = (SAMPLE_RATE as f64 * duration_sec).round() as u32;
    write_mono_wav(
        path,
        (0..frames).map(|frame| sine_sample(frame, 1_000.0, 0.4)),
    );
}

fn truncate_wav_data_to_half(path: &Path) {
    let bytes = std::fs::read(path).expect("read complete wav");
    assert!(
        bytes.len() > 44,
        "fixture should include a WAV header and data"
    );
    let truncated_len = 44 + (bytes.len() - 44) / 2;
    std::fs::write(path, &bytes[..truncated_len]).expect("write truncated wav");
}

fn float_to_i16(sample: f32) -> i16 {
    (sample.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16
}

fn sine_sample(frame: u32, freq_hz: f32, amplitude: f32) -> f32 {
    let phase = 2.0 * std::f32::consts::PI * freq_hz * frame as f32 / SAMPLE_RATE as f32;
    amplitude * phase.sin()
}

#[test]
fn clean_fixture_matches_full_json_contract() {
    let dir = FixtureDir::new("contract");
    let path = dir.path().join("clean_gap.wav");
    write_clean_gap_wav(&path);

    let json = audioscan_json(&path, false);
    let object = json
        .as_object()
        .expect("audioscan output should be an object");
    let actual_fields = object.keys().map(String::as_str).collect::<BTreeSet<_>>();
    let expected_fields = CONTRACT_FIELDS.into_iter().collect::<BTreeSet<_>>();

    assert_eq!(actual_fields, expected_fields);
    assert_eq!(field(&json, "schema_version").as_u64(), Some(1));
    assert_clean_ok(&json);
    assert_eq!(field(&json, "skipped_packets").as_u64(), Some(0));
}

#[test]
fn truncated_wav_reports_partial_status_warning_and_half_duration() {
    let dir = FixtureDir::new("truncated-partial");
    let path = dir.path().join("truncated.wav");
    let original_duration = 4.0;
    write_tone_wav(&path, original_duration);
    truncate_wav_data_to_half(&path);

    let output = audioscan_output(&path, false);
    assert_eq!(
        output.status.code(),
        Some(0),
        "truncated input should be a non-strict success\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let json: Value = serde_json::from_slice(&output.stdout).expect("stdout JSON");
    let warnings = warnings(&json);

    assert_eq!(field(&json, "status").as_str(), Some("partial"));
    assert!(
        !warnings.is_empty(),
        "partial decode should include warnings"
    );
    assert!(
        warnings
            .iter()
            .filter_map(Value::as_str)
            .any(|warning| warning.starts_with("truncated:")),
        "warnings should include a truncated: entry, got {warnings:?}"
    );
    assert_approx(
        number_field(&json, "duration_sec"),
        original_duration / 2.0,
        0.15,
        "duration_sec",
    );
}

#[test]
fn strict_truncated_wav_exits_with_error() {
    let dir = FixtureDir::new("truncated-strict");
    let path = dir.path().join("truncated.wav");
    write_tone_wav(&path, 4.0);
    truncate_wav_data_to_half(&path);

    let output = audioscan_output(&path, true);
    assert_eq!(
        output.status.code(),
        Some(1),
        "strict truncated input should exit 1\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr).to_lowercase();
    assert!(
        stderr.contains("strict"),
        "strict error should mention strict mode, got:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn strict_clean_wav_matches_non_strict_success_semantics() {
    let dir = FixtureDir::new("strict-clean");
    let path = dir.path().join("clean_gap.wav");
    write_clean_gap_wav(&path);

    let non_strict = audioscan_json(&path, false);
    let strict = audioscan_json(&path, true);

    assert_clean_ok(&non_strict);
    assert!(warnings(&non_strict).is_empty());
    assert_clean_ok(&strict);
    assert!(warnings(&strict).is_empty());
}
