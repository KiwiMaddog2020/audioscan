use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use hound::{SampleFormat, WavSpec, WavWriter};
use serde_json::Value;

const DEFAULT_SAMPLE_RATE: u32 = 48_000;
static NEXT_FIXTURE_DIR: AtomicUsize = AtomicUsize::new(0);

struct FixtureDir {
    path: PathBuf,
}

impl FixtureDir {
    fn new(name: &str) -> Self {
        let unique = NEXT_FIXTURE_DIR.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "audioscan-robustness-{name}-{}-{unique}",
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

fn audioscan_json(path: &Path) -> Value {
    let output = Command::new(env!("CARGO_BIN_EXE_audioscan"))
        .arg("--compact")
        .arg(path)
        .output()
        .expect("run audioscan");
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

fn assert_finite_negative(json: &Value, name: &str) {
    let value = number_field(json, name);
    assert!(value.is_finite(), "{name} should be finite");
    assert!(value < 0.0, "{name} should be negative, got {value}");
}

fn write_mono_i16_tone(path: &Path, sample_rate: u32, duration_sec: f64) {
    let spec = WavSpec {
        channels: 1,
        sample_rate,
        bits_per_sample: 16,
        sample_format: SampleFormat::Int,
    };
    let mut writer = WavWriter::create(path, spec).expect("create mono i16 wav");
    for frame in 0..frames_for(sample_rate, duration_sec) {
        writer
            .write_sample(float_to_i16(sine_sample(frame, sample_rate, 1_000.0, 0.4)))
            .expect("write mono i16 sample");
    }
    writer.finalize().expect("finalize mono i16 wav");
}

fn write_stereo_i16_in_phase_tone(path: &Path, duration_sec: f64) {
    let spec = WavSpec {
        channels: 2,
        sample_rate: DEFAULT_SAMPLE_RATE,
        bits_per_sample: 16,
        sample_format: SampleFormat::Int,
    };
    let mut writer = WavWriter::create(path, spec).expect("create stereo i16 wav");
    for frame in 0..frames_for(DEFAULT_SAMPLE_RATE, duration_sec) {
        let sample = float_to_i16(sine_sample(frame, DEFAULT_SAMPLE_RATE, 1_000.0, 0.4));
        writer.write_sample(sample).expect("write left sample");
        writer.write_sample(sample).expect("write right sample");
    }
    writer.finalize().expect("finalize stereo i16 wav");
}

fn write_mono_i24_tone(path: &Path, duration_sec: f64) {
    let spec = WavSpec {
        channels: 1,
        sample_rate: DEFAULT_SAMPLE_RATE,
        bits_per_sample: 24,
        sample_format: SampleFormat::Int,
    };
    let mut writer = WavWriter::create(path, spec).expect("create mono i24 wav");
    for frame in 0..frames_for(DEFAULT_SAMPLE_RATE, duration_sec) {
        writer
            .write_sample(float_to_i24(sine_sample(
                frame,
                DEFAULT_SAMPLE_RATE,
                1_000.0,
                0.4,
            )))
            .expect("write mono i24 sample");
    }
    writer.finalize().expect("finalize mono i24 wav");
}

fn frames_for(sample_rate: u32, duration_sec: f64) -> u32 {
    (sample_rate as f64 * duration_sec).round() as u32
}

fn float_to_i16(sample: f32) -> i16 {
    (sample.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16
}

fn float_to_i24(sample: f32) -> i32 {
    (sample.clamp(-1.0, 1.0) * 8_388_607.0).round() as i32
}

fn sine_sample(frame: u32, sample_rate: u32, freq_hz: f32, amplitude: f32) -> f32 {
    let phase = 2.0 * std::f32::consts::PI * freq_hz * frame as f32 / sample_rate as f32;
    amplitude * phase.sin()
}

#[test]
fn wav_at_44100_hz_scans_ok() {
    let dir = FixtureDir::new("44100");
    let path = dir.path().join("tone_44100.wav");
    write_mono_i16_tone(&path, 44_100, 1.0);

    let json = audioscan_json(&path);

    assert_eq!(field(&json, "status").as_str(), Some("ok"));
    assert_eq!(field(&json, "sample_rate").as_u64(), Some(44_100));
}

#[test]
fn in_phase_stereo_content_reports_loudness() {
    let dir = FixtureDir::new("stereo");
    let path = dir.path().join("stereo_in_phase.wav");
    write_stereo_i16_in_phase_tone(&path, 2.0);

    let json = audioscan_json(&path);

    assert_eq!(field(&json, "status").as_str(), Some("ok"));
    assert_eq!(field(&json, "channels").as_u64(), Some(2));
    assert_finite_negative(&json, "integrated_lufs");
}

#[test]
fn twenty_four_bit_wav_reports_bit_depth() {
    let dir = FixtureDir::new("i24");
    let path = dir.path().join("tone_24bit.wav");
    write_mono_i24_tone(&path, 1.0);

    let json = audioscan_json(&path);

    assert_eq!(field(&json, "status").as_str(), Some("ok"));
    assert_eq!(field(&json, "bits_per_sample").as_u64(), Some(24));
}
