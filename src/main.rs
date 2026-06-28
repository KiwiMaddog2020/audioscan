//! audioscan CLI: a thin wrapper over the `audioscan` library.
//!
//!     audioscan [--compact] [--strict] [--threshold <dB>] [--min-gap <s>] <file>
//!     audioscan batch <dir> [--out <file.jsonl>] [--jobs auto|<N>] [--strict] ...
//!
//! Single-file analysis lives in `audioscan::analyze_path`. `batch` fans that
//! out across a directory tree with rayon and writes one JSON object per line,
//! isolating each file so a single bad recording cannot abort the run.

use std::fs::{self, File};
use std::io::{self, BufWriter, Write};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use audioscan::{Analysis, SCHEMA_VERSION, ScanConfig, ScanError, Status, analyze_path};
use rayon::prelude::*;
use serde::Serialize;
use serde_json::json;

const USAGE: &str = "\
usage:
  audioscan [--compact] [--strict] [--threshold <dB>] [--min-gap <s>] [--timeout <s>] <file>
  audioscan batch <dir> [--out <file.jsonl>] [--jobs auto|<N>] [--strict] [--threshold <dB>] [--min-gap <s>] [--timeout <s>]";

const HELP: &str = "\
audioscan — decode an audio file once and report loudness, silence, and format as JSON.

USAGE:
  audioscan [FLAGS] <file>
  audioscan batch <dir> [FLAGS]

FLAGS:
  --compact          one-line JSON (single-file default is pretty; batch is always compact)
  --pretty           pretty-printed JSON (single-file only)
  --strict           exit non-zero instead of a \"partial\" result on an incomplete decode
  --threshold <dB>   silence threshold in RMS dBFS (default -30)
  --min-gap <s>      shortest silence to report, in seconds (default 5.0)
  --timeout <s>      per-file soft decode deadline in seconds (default: none)
  -h, --help         print this help
  -V, --version      print version

BATCH FLAGS:
  --out <file>       write JSON Lines to a file (default: stdout)
  --jobs auto|<N>    parallel worker count (default: auto = cpu cores)";

const AUDIO_EXTS: &[&str] = &[
    "wav", "wave", "mp3", "m4a", "mp4", "aac", "flac", "ogg", "oga", "aif", "aiff", "mka", "opus",
];

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("batch") {
        cmd_batch(&args[1..])
    } else {
        cmd_single(&args)
    }
}

fn usage_err(msg: &str) -> ExitCode {
    eprintln!("audioscan: {msg}\n{USAGE}");
    ExitCode::from(2)
}

fn parse_f64_flag(value: Option<&String>, what: &str) -> Result<f64, String> {
    value
        .ok_or_else(|| format!("{what} needs a value"))?
        .parse()
        .map_err(|_| format!("{what} must be a number"))
}

// ---------------------------------------------------------------------------
// Single-file mode
// ---------------------------------------------------------------------------

fn cmd_single(args: &[String]) -> ExitCode {
    let mut config = ScanConfig::default();
    let mut pretty = true;
    let mut path: Option<String> = None;

    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--compact" => pretty = false,
            "--pretty" => pretty = true,
            "--strict" => config.strict = true,
            "--threshold" => match parse_f64_flag(it.next(), "--threshold") {
                Ok(v) => config.threshold_db = v,
                Err(e) => return usage_err(&e),
            },
            "--min-gap" => match parse_f64_flag(it.next(), "--min-gap") {
                Ok(v) => config.min_gap_sec = v,
                Err(e) => return usage_err(&e),
            },
            "--timeout" => match parse_f64_flag(it.next(), "--timeout") {
                Ok(v) => config.max_decode_secs = Some(v),
                Err(e) => return usage_err(&e),
            },
            "-h" | "--help" => {
                println!("{HELP}");
                return ExitCode::SUCCESS;
            }
            "-V" | "--version" => {
                println!("audioscan {}", env!("CARGO_PKG_VERSION"));
                return ExitCode::SUCCESS;
            }
            flag if flag.starts_with('-') => return usage_err(&format!("unknown flag: {flag}")),
            file => {
                if path.is_some() {
                    return usage_err("only one input file is supported");
                }
                path = Some(file.to_string());
            }
        }
    }

    if let Err(e) = config.validate() {
        return usage_err(&e.to_string());
    }
    let path = match path {
        Some(p) => p,
        None => return usage_err("no input file (usage: audioscan <file>)"),
    };

    let started = Instant::now();
    match analyze_path(&path, &config) {
        Ok(analysis) => {
            let json = if pretty {
                serde_json::to_string_pretty(&analysis)
            } else {
                serde_json::to_string(&analysis)
            };
            match json {
                Ok(s) => {
                    println!("{s}");
                    eprintln!(
                        "audioscan: analyzed {path} in {:.2}s",
                        started.elapsed().as_secs_f64()
                    );
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("audioscan: could not serialize result: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        Err(e) => {
            eprintln!("audioscan: {e}");
            ExitCode::FAILURE
        }
    }
}

// ---------------------------------------------------------------------------
// Batch mode
// ---------------------------------------------------------------------------

/// One batch result: the analysis (or error) plus per-file telemetry. `bytes`
/// is the input file's size on disk; `elapsed_ms` is the wall-clock time this
/// file took to analyze, surfaced in the stderr diagnostics.
struct BatchRow {
    path: PathBuf,
    result: Result<Analysis, ScanError>,
    elapsed_ms: u128,
    bytes: u64,
}

/// Serializable batch JSONL row: the analysis fields, flattened, plus the
/// deterministic per-file `bytes` telemetry. Flatten keeps the field order
/// identical to the single-file object, with `bytes` appended last.
///
/// Serialize-only: a batch row is a superset of `Analysis` (the extra `bytes`
/// key), so it is not meant to be deserialized back into `Analysis`, which is
/// `deny_unknown_fields`. Parse the single-file output for that.
#[derive(Serialize)]
struct BatchRecord<'a> {
    #[serde(flatten)]
    analysis: &'a Analysis,
    bytes: u64,
}

fn cmd_batch(args: &[String]) -> ExitCode {
    let started = Instant::now();
    let mut config = ScanConfig::default();
    let mut dir: Option<String> = None;
    let mut out: Option<String> = None;
    let mut jobs: Option<usize> = None;

    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--compact" => {} // batch always emits compact JSON Lines
            "--strict" => config.strict = true,
            "--out" => match it.next() {
                Some(v) => out = Some(v.clone()),
                None => return usage_err("--out needs a file path"),
            },
            "--jobs" => match it.next().map(String::as_str) {
                Some("auto") => jobs = None,
                Some(v) => match v.parse::<usize>() {
                    Ok(n) if n >= 1 => jobs = Some(n),
                    _ => return usage_err("--jobs must be 'auto' or a positive integer"),
                },
                None => return usage_err("--jobs needs a value"),
            },
            "--threshold" => match parse_f64_flag(it.next(), "--threshold") {
                Ok(v) => config.threshold_db = v,
                Err(e) => return usage_err(&e),
            },
            "--min-gap" => match parse_f64_flag(it.next(), "--min-gap") {
                Ok(v) => config.min_gap_sec = v,
                Err(e) => return usage_err(&e),
            },
            "--timeout" => match parse_f64_flag(it.next(), "--timeout") {
                Ok(v) => config.max_decode_secs = Some(v),
                Err(e) => return usage_err(&e),
            },
            "-h" | "--help" => {
                println!("{HELP}");
                return ExitCode::SUCCESS;
            }
            flag if flag.starts_with('-') => return usage_err(&format!("unknown flag: {flag}")),
            d => {
                if dir.is_some() {
                    return usage_err("only one input directory is supported");
                }
                dir = Some(d.to_string());
            }
        }
    }

    if let Err(e) = config.validate() {
        return usage_err(&e.to_string());
    }
    let dir = match dir {
        Some(d) => d,
        None => return usage_err("no input directory (usage: audioscan batch <dir>)"),
    };

    let mut files = collect_audio_files(Path::new(&dir));
    files.sort();
    if files.is_empty() {
        eprintln!("audioscan: no audio files found under {dir}");
        return ExitCode::from(1);
    }

    let total = files.len();
    let done = AtomicUsize::new(0);
    let scan = || -> Vec<BatchRow> {
        files
            .par_iter()
            .map(|p| {
                let bytes = fs::metadata(p).map(|m| m.len()).unwrap_or(0);
                let started = Instant::now();
                // Isolate each file: a panic in one decode becomes an error row,
                // never an aborted batch.
                let result = catch_unwind(AssertUnwindSafe(|| analyze_path(p, &config)))
                    .unwrap_or_else(|_| {
                        Err(ScanError::Decode("panicked while decoding".to_string()))
                    });
                let elapsed_ms = started.elapsed().as_millis();
                // Live progress: stream a per-file breadcrumb to stderr as each
                // file finishes, so a wedged or slow file is visible mid-run and
                // the batch is not silent until every decode returns. stderr only,
                // so stdout JSON Lines stay byte-identical across --jobs counts.
                let k = done.fetch_add(1, Ordering::Relaxed) + 1;
                eprintln!("audioscan: [{k}/{total}] {} ({elapsed_ms}ms)", p.display());
                BatchRow {
                    path: p.clone(),
                    result,
                    elapsed_ms,
                    bytes,
                }
            })
            .collect()
    };

    let mut results = match jobs {
        Some(n) => match rayon::ThreadPoolBuilder::new().num_threads(n).build() {
            Ok(pool) => pool.install(scan),
            Err(e) => {
                eprintln!("audioscan: could not build thread pool: {e}");
                return ExitCode::FAILURE;
            }
        },
        None => scan(),
    };
    results.sort_by(|a, b| a.path.cmp(&b.path));

    let mut sink: Box<dyn Write> = match &out {
        Some(p) => match File::create(p) {
            Ok(f) => Box::new(BufWriter::new(f)),
            Err(e) => {
                eprintln!("audioscan: could not create {p}: {e}");
                return ExitCode::FAILURE;
            }
        },
        None => Box::new(BufWriter::new(io::stdout())),
    };

    let (mut ok, mut partial, mut failed) = (0u64, 0u64, 0u64);
    for row in &results {
        let path_str = row.path.to_string_lossy();
        let line = match &row.result {
            Ok(analysis) => {
                if analysis.status == Status::Partial {
                    partial += 1;
                } else {
                    ok += 1;
                }
                let record = BatchRecord {
                    analysis,
                    bytes: row.bytes,
                };
                serde_json::to_string(&record).unwrap_or_else(|e| {
                    json!({"schema_version": SCHEMA_VERSION, "path": path_str, "error": format!("serialize: {e}"), "bytes": row.bytes})
                        .to_string()
                })
            }
            Err(e) => {
                failed += 1;
                json!({"schema_version": SCHEMA_VERSION, "path": path_str, "error": e.to_string(), "bytes": row.bytes})
                    .to_string()
            }
        };
        if let Err(e) = writeln!(sink, "{line}") {
            eprintln!("audioscan: write failed: {e}");
            return ExitCode::FAILURE;
        }
    }
    if let Err(e) = sink.flush() {
        eprintln!("audioscan: flush failed: {e}");
        return ExitCode::FAILURE;
    }

    eprintln!(
        "audioscan: scanned {} file(s): {ok} ok, {partial} partial, {failed} failed in {:.1}s",
        results.len(),
        started.elapsed().as_secs_f64()
    );
    eprintln!("audioscan: slowest: {}", slowest_report(&results));
    ExitCode::SUCCESS
}

/// Format the slowest few files for the batch stderr diagnostic, longest first,
/// so a pathologically slow file in a large run is individually visible without
/// scanning the JSON Lines. Per-file wall-clock timing stays on stderr because
/// stdout is byte-deterministic across `--jobs` counts.
fn slowest_report(results: &[BatchRow]) -> String {
    let mut by_time: Vec<&BatchRow> = results.iter().collect();
    by_time.sort_by_key(|r| std::cmp::Reverse(r.elapsed_ms));
    by_time
        .iter()
        .take(3)
        .map(|r| {
            let name = r.path.file_name().and_then(|n| n.to_str()).unwrap_or("?");
            format!("{name} {}ms ({})", r.elapsed_ms, human_bytes(r.bytes))
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Compact human-readable byte size, e.g. `8.1 MB`.
fn human_bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut size = n as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{n} B")
    } else {
        format!("{size:.1} {}", UNITS[unit])
    }
}

/// Recursively collect files with a known audio extension under `dir`.
fn collect_audio_files(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let entries = match fs::read_dir(&d) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let p = entry.path();
            // Entry type does NOT follow symlinks, so a symlinked directory
            // can't cause an unbounded loop or escape the requested root.
            let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
            if is_dir {
                stack.push(p);
            } else if let Some(ext) = p.extension().and_then(|e| e.to_str())
                && AUDIO_EXTS.contains(&ext.to_lowercase().as_str())
            {
                found.push(p);
            }
        }
    }
    found
}
