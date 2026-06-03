use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

fn fixture_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
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

#[test]
fn flac_fixture_reports_codec_status_duration_and_loudness() {
    let path = fixture_path("tone_1khz_1s.flac");

    let json = audioscan_json(&path);

    assert_eq!(field(&json, "codec").as_str(), Some("flac"));
    assert_eq!(field(&json, "status").as_str(), Some("ok"));
    assert_approx(
        number_field(&json, "duration_sec"),
        1.0,
        0.02,
        "duration_sec",
    );
    assert_finite_negative(&json, "integrated_lufs");
}

#[test]
fn lame_mp3_fixture_reports_gapless_one_second_duration() {
    let path = fixture_path("tone_1khz_1s_gapless.mp3");

    let json = audioscan_json(&path);

    assert_eq!(field(&json, "codec").as_str(), Some("mp3"));
    assert_eq!(field(&json, "status").as_str(), Some("ok"));
    assert_approx(
        number_field(&json, "duration_sec"),
        1.0,
        0.01,
        "duration_sec",
    );
}
