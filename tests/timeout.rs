//! `--timeout` soft decode-deadline behaviour, driven through the CLI.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

use hound::{SampleFormat, WavSpec, WavWriter};
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
            "audioscan-timeout-{name}-{}-{unique}",
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

/// Write a mono tone of `duration_sec`. The tests use a multi-second tone so a
/// 1 ms `--timeout` trips with a wide margin (the decode is reliably tens of ms),
/// keeping the timing assertions robust across machines.
fn write_tone(path: &Path, duration_sec: f64) {
    let spec = WavSpec {
        channels: 1,
        sample_rate: SAMPLE_RATE,
        bits_per_sample: 16,
        sample_format: SampleFormat::Int,
    };
    let mut writer = WavWriter::create(path, spec).expect("create wav");
    let frames = (SAMPLE_RATE as f64 * duration_sec) as u32;
    for frame in 0..frames {
        let phase = 2.0 * std::f32::consts::PI * 440.0 * frame as f32 / SAMPLE_RATE as f32;
        let sample = (0.4 * phase.sin()).clamp(-1.0, 1.0) * i16::MAX as f32;
        writer
            .write_sample(sample.round() as i16)
            .expect("write sample");
    }
    writer.finalize().expect("finalize wav");
}

fn run(path: &Path, extra: &[&str]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_audioscan"));
    command.arg("--compact");
    for arg in extra {
        command.arg(arg);
    }
    command.arg(path).output().expect("run audioscan")
}

fn warnings(json: &Value) -> &Vec<Value> {
    json.get("warnings")
        .and_then(Value::as_array)
        .expect("warnings should be a JSON array")
}

#[test]
fn tiny_timeout_yields_partial_with_timeout_warning() {
    let dir = FixtureDir::new("trip");
    let path = dir.path().join("long.wav");
    write_tone(&path, 10.0);

    let output = run(&path, &["--timeout", "0.001"]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "a soft timeout is a non-strict success\nstderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: Value = serde_json::from_slice(&output.stdout).expect("stdout JSON");
    assert_eq!(json.get("status").and_then(Value::as_str), Some("partial"));
    assert!(
        warnings(&json)
            .iter()
            .filter_map(Value::as_str)
            .any(|w| w.contains("timeout")),
        "expected a timeout warning, got {:?}",
        warnings(&json)
    );
}

#[test]
fn strict_tiny_timeout_exits_with_error() {
    let dir = FixtureDir::new("strict");
    let path = dir.path().join("long.wav");
    write_tone(&path, 10.0);

    let output = run(&path, &["--strict", "--timeout", "0.001"]);
    assert_eq!(
        output.status.code(),
        Some(1),
        "a strict timeout should exit 1\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn generous_timeout_completes_cleanly() {
    let dir = FixtureDir::new("clean");
    let path = dir.path().join("short.wav");
    write_tone(&path, 1.0);

    let output = run(&path, &["--timeout", "60"]);
    assert_eq!(output.status.code(), Some(0));
    let json: Value = serde_json::from_slice(&output.stdout).expect("stdout JSON");
    assert_eq!(json.get("status").and_then(Value::as_str), Some("ok"));
    assert!(
        warnings(&json).is_empty(),
        "a generous timeout should not warn: {:?}",
        warnings(&json)
    );
}

#[test]
fn invalid_timeout_values_are_usage_errors() {
    let dir = FixtureDir::new("bad");
    let path = dir.path().join("short.wav");
    write_tone(&path, 0.2);

    for bad in [
        ["--timeout", "0"],
        ["--timeout", "-1"],
        ["--timeout", "nan"],
    ] {
        let output = run(&path, &bad);
        assert_eq!(
            output.status.code(),
            Some(2),
            "{bad:?} should be a usage error\nstderr:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
