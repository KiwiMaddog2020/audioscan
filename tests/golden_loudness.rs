use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::Value;

const SAMPLE_RATE: u32 = 48_000;
const SEGMENT_SECONDS: u32 = 4;
const LEVELS_DBFS: [f64; 5] = [-6.0, -18.0, -3.0, -14.0, -9.0];

static NEXT_FIXTURE_DIR: AtomicUsize = AtomicUsize::new(0);

#[derive(Debug)]
struct Loudness {
    integrated_lufs: f64,
    loudness_range_lu: f64,
    true_peak_db: f64,
}

struct FixtureDir {
    path: PathBuf,
}

impl FixtureDir {
    fn new(name: &str) -> Self {
        let unique = NEXT_FIXTURE_DIR.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "audioscan-golden-loudness-{name}-{}-{unique}",
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

fn find_executable(name: &str) -> Option<PathBuf> {
    let paths = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&paths) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn assert_success(output: &Output, action: &str) {
    assert!(
        output.status.success(),
        "{action} failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn varied_signal_filter() -> String {
    let mut filters = Vec::new();
    let mut labels = String::new();

    for (index, dbfs) in LEVELS_DBFS.iter().enumerate() {
        let amplitude = 10.0_f64.powf(dbfs / 20.0);
        filters.push(format!(
            "aevalsrc=exprs={amplitude:.17}*sin(2*PI*1000*t):s={SAMPLE_RATE}:d={SEGMENT_SECONDS}[a{index}]"
        ));
        labels.push_str(&format!("[a{index}]"));
    }

    filters.push(format!("{labels}concat=n={}:v=0:a=1", LEVELS_DBFS.len()));
    filters.join(";")
}

fn generate_varied_signal(ffmpeg: &Path, path: &Path) {
    let output = Command::new(ffmpeg)
        .arg("-hide_banner")
        .arg("-nostats")
        .arg("-y")
        .arg("-f")
        .arg("lavfi")
        .arg("-i")
        .arg(varied_signal_filter())
        .arg("-ac")
        .arg("1")
        .arg("-ar")
        .arg(SAMPLE_RATE.to_string())
        .arg("-c:a")
        .arg("pcm_s16le")
        .arg(path)
        .output()
        .expect("run ffmpeg to generate varied signal");
    assert_success(&output, "ffmpeg lavfi signal generation");
}

fn audioscan_loudness(audioscan: &Path, path: &Path) -> Loudness {
    let output = Command::new(audioscan)
        .arg("--compact")
        .arg(path)
        .output()
        .expect("run audioscan");
    assert_success(&output, "audioscan");

    let json: Value = serde_json::from_slice(&output.stdout).expect("audioscan stdout is JSON");
    Loudness {
        integrated_lufs: number_field(&json, "integrated_lufs"),
        loudness_range_lu: number_field(&json, "loudness_range_lu"),
        true_peak_db: number_field(&json, "true_peak_dbtp"),
    }
}

fn number_field(json: &Value, name: &str) -> f64 {
    json.get(name)
        .unwrap_or_else(|| panic!("missing JSON field {name}"))
        .as_f64()
        .unwrap_or_else(|| panic!("{name} should be a JSON number"))
}

fn ffmpeg_ebur128_loudness(ffmpeg: &Path, path: &Path) -> Loudness {
    let output = Command::new(ffmpeg)
        .arg("-hide_banner")
        .arg("-nostats")
        .arg("-i")
        .arg(path)
        // Newer ffmpeg only emits the Summary "True peak" block when peak
        // reporting is enabled on the ebur128 filter.
        .arg("-af")
        .arg("ebur128=peak=true")
        .arg("-f")
        .arg("null")
        .arg("-")
        .output()
        .expect("run ffmpeg ebur128");
    assert_success(&output, "ffmpeg ebur128");

    let stderr = String::from_utf8_lossy(&output.stderr);
    parse_ffmpeg_summary(&stderr).unwrap_or_else(|| {
        panic!(
            "ffmpeg ebur128 Summary block was missing or incomplete\nstderr:\n{}",
            stderr
        )
    })
}

fn parse_ffmpeg_summary(stderr: &str) -> Option<Loudness> {
    let (_, summary) = stderr.rsplit_once("Summary:")?;
    Some(Loudness {
        integrated_lufs: parse_summary_number(summary, "I:", "LUFS")?,
        loudness_range_lu: parse_summary_number(summary, "LRA:", "LU")?,
        true_peak_db: parse_summary_number(summary, "Peak:", "dBFS")?,
    })
}

fn parse_summary_number(summary: &str, label: &str, unit: &str) -> Option<f64> {
    for line in summary.lines() {
        let Some(rest) = line.trim().strip_prefix(label) else {
            continue;
        };
        let mut parts = rest.split_whitespace();
        let value = parts.next()?;
        let actual_unit = parts.next()?;

        if actual_unit == unit {
            return value.parse().ok();
        }
    }

    None
}

fn assert_close(metric: &str, audioscan: f64, ffmpeg: f64, tolerance: f64) {
    let delta = (audioscan - ffmpeg).abs();
    assert!(
        delta <= tolerance,
        "{metric}: audioscan={audioscan:.3}, ffmpeg={ffmpeg:.3}, delta={delta:.3}, tolerance={tolerance:.3}"
    );
}

#[test]
fn varied_signal_matches_ffmpeg_ebur128_golden_values() {
    let Some(ffmpeg) = find_executable("ffmpeg") else {
        eprintln!("skipping golden loudness test: ffmpeg not found on PATH");
        return;
    };

    let audioscan = PathBuf::from(env!("CARGO_BIN_EXE_audioscan"));
    if !audioscan.is_file() {
        eprintln!(
            "skipping golden loudness test: audioscan test binary not found at {}",
            audioscan.display()
        );
        return;
    }

    let dir = FixtureDir::new("varied");
    let signal = dir.path().join("varied_levels.wav");
    generate_varied_signal(&ffmpeg, &signal);

    let audioscan = audioscan_loudness(&audioscan, &signal);
    let ffmpeg = ffmpeg_ebur128_loudness(&ffmpeg, &signal);

    eprintln!(
        "golden loudness deltas: integrated={:.3} LU, LRA={:.3} LU, true_peak={:.3} dB",
        (audioscan.integrated_lufs - ffmpeg.integrated_lufs).abs(),
        (audioscan.loudness_range_lu - ffmpeg.loudness_range_lu).abs(),
        (audioscan.true_peak_db - ffmpeg.true_peak_db).abs()
    );

    // README validation observed about 0.04 LU on this varied signal; 0.3 LU
    // is a narrow regression band while allowing ffmpeg/library drift.
    assert_close(
        "integrated_lufs",
        audioscan.integrated_lufs,
        ffmpeg.integrated_lufs,
        0.3,
    );

    // LRA is only asserted on a varied-level signal; percentile gating can
    // differ slightly across implementations, so this tolerance is wider.
    assert_close(
        "loudness_range_lu",
        audioscan.loudness_range_lu,
        ffmpeg.loudness_range_lu,
        1.5,
    );

    // audioscan reports dBTP while ffmpeg labels ebur128 true peak as dBFS;
    // oversampling implementations are expected to be close, not identical.
    assert_close(
        "true_peak",
        audioscan.true_peak_db,
        ffmpeg.true_peak_db,
        0.6,
    );
}
