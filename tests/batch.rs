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
            "audioscan-batch-{name}-{}-{unique}",
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

fn audioscan_batch(dir: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_audioscan"));
    command.arg("batch").arg(dir);
    command
}

fn create_batch_fixture(name: &str) -> FixtureDir {
    let dir = FixtureDir::new(name);
    let subdir = dir.path().join("sub");
    std::fs::create_dir_all(&subdir).expect("create sub fixture dir");

    write_tone_wav(&dir.path().join("a.wav"), 440.0, 1.0);
    write_tone_wav(&subdir.join("b.wav"), 880.0, 1.0);
    std::fs::write(dir.path().join("bad.wav"), b"RIFFxxxxWAVEjunk").expect("write bad wav");
    write_truncated_wav(&dir.path().join("trunc.wav"));

    dir
}

fn write_tone_wav(path: &Path, freq_hz: f32, duration_sec: f64) {
    let frames = (SAMPLE_RATE as f64 * duration_sec).round() as u32;
    write_mono_wav(
        path,
        (0..frames).map(|frame| sine_sample(frame, freq_hz, 0.4)),
    );
}

fn write_truncated_wav(path: &Path) {
    write_tone_wav(path, 220.0, 4.0);
    let bytes = std::fs::read(path).expect("read complete wav");
    assert!(
        bytes.len() > 44,
        "fixture should include a WAV header and data"
    );
    let data_bytes_to_keep = ((bytes.len() - 44) / 2).max(2);
    std::fs::write(path, &bytes[..44 + data_bytes_to_keep]).expect("write truncated wav");
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

fn float_to_i16(sample: f32) -> i16 {
    (sample.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16
}

fn sine_sample(frame: u32, freq_hz: f32, amplitude: f32) -> f32 {
    let phase = 2.0 * std::f32::consts::PI * freq_hz * frame as f32 / SAMPLE_RATE as f32;
    amplitude * phase.sin()
}

fn assert_success(output: &Output, label: &str) {
    assert_eq!(
        output.status.code(),
        Some(0),
        "{label} should exit 0\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn parse_jsonl(jsonl: &str) -> Vec<Value> {
    jsonl
        .lines()
        .map(|line| serde_json::from_str(line).expect("JSONL line is valid JSON"))
        .collect()
}

fn field<'a>(json: &'a Value, name: &str) -> &'a Value {
    json.get(name)
        .unwrap_or_else(|| panic!("missing JSON field {name}"))
}

fn row_for_basename<'a>(rows: &'a [Value], basename: &str) -> &'a Value {
    let matches = rows
        .iter()
        .filter(|row| {
            field(row, "path")
                .as_str()
                .and_then(|path| Path::new(path).file_name())
                .and_then(|name| name.to_str())
                == Some(basename)
        })
        .collect::<Vec<_>>();
    assert_eq!(
        matches.len(),
        1,
        "{basename} should appear exactly once in {rows:#?}"
    );
    matches[0]
}

#[test]
fn batch_writes_jsonl_for_success_error_and_partial_rows() {
    let dir = create_batch_fixture("mixed-out");
    let out_path = dir.path().join("batch.jsonl");

    let output = audioscan_batch(dir.path())
        .arg("--out")
        .arg(&out_path)
        .output()
        .expect("run audioscan batch with --out");
    assert_success(&output, "batch --out");

    let jsonl = std::fs::read_to_string(&out_path).expect("read batch JSONL");
    let rows = parse_jsonl(&jsonl);
    assert_eq!(
        rows.len(),
        4,
        "JSONL should contain exactly 4 rows:\n{jsonl}"
    );

    let a = row_for_basename(&rows, "a.wav");
    let b = row_for_basename(&rows, "b.wav");
    let bad = row_for_basename(&rows, "bad.wav");
    let trunc = row_for_basename(&rows, "trunc.wav");

    assert_eq!(field(a, "status").as_str(), Some("ok"));
    assert_eq!(field(b, "status").as_str(), Some("ok"));

    assert_eq!(field(bad, "schema_version").as_u64(), Some(1));
    assert!(
        bad.get("error").and_then(Value::as_str).is_some(),
        "bad.wav should include an error string: {bad:#?}"
    );
    assert!(
        bad.get("status").is_none(),
        "bad.wav should not have status"
    );
    assert!(
        bad.get("silences").is_none(),
        "bad.wav should not have silences"
    );

    assert_eq!(field(trunc, "status").as_str(), Some("partial"));

    // Every row (success and error) carries the deterministic `bytes` telemetry.
    for (row, label) in [
        (a, "a.wav"),
        (b, "b.wav"),
        (bad, "bad.wav"),
        (trunc, "trunc.wav"),
    ] {
        let bytes = field(row, "bytes")
            .as_u64()
            .unwrap_or_else(|| panic!("{label} row should carry a numeric bytes field"));
        assert!(bytes > 0, "{label} bytes should be > 0");
    }
    // The two clean WAVs are real files larger than a bare 44-byte header.
    assert!(field(a, "bytes").as_u64().unwrap() > 44);
    assert!(field(b, "bytes").as_u64().unwrap() > 44);
}

#[test]
fn batch_stdout_is_identical_for_single_and_multi_job_runs() {
    let dir = create_batch_fixture("jobs");

    let single = audioscan_batch(dir.path())
        .args(["--jobs", "1"])
        .output()
        .expect("run audioscan batch --jobs 1");
    let multi = audioscan_batch(dir.path())
        .args(["--jobs", "4"])
        .output()
        .expect("run audioscan batch --jobs 4");

    assert_success(&single, "batch --jobs 1");
    assert_success(&multi, "batch --jobs 4");
    assert_eq!(
        single.stdout,
        multi.stdout,
        "batch stdout should be byte-identical across job counts\n--jobs 1:\n{}\n--jobs 4:\n{}",
        String::from_utf8_lossy(&single.stdout),
        String::from_utf8_lossy(&multi.stdout)
    );
}

#[test]
fn batch_stderr_reports_summary_and_slowest_timing() {
    let dir = create_batch_fixture("stderr-timing");

    let output = audioscan_batch(dir.path())
        .output()
        .expect("run audioscan batch to stdout");
    assert_success(&output, "batch stderr");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("scanned 4 file(s)"),
        "stderr should summarize the run, got:\n{stderr}"
    );
    // Per-file timing is observable on stderr (stdout stays byte-deterministic).
    assert!(
        stderr.contains("slowest:") && stderr.contains("ms"),
        "stderr should report the slowest files with per-file ms timing, got:\n{stderr}"
    );
}

#[test]
fn batch_empty_audio_dir_exits_one() {
    let dir = FixtureDir::new("empty");
    std::fs::write(dir.path().join("notes.txt"), b"not audio").expect("write non-audio file");

    let output = audioscan_batch(dir.path())
        .output()
        .expect("run audioscan batch on empty audio dir");

    assert_eq!(
        output.status.code(),
        Some(1),
        "no-audio batch should exit 1\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
