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

use audioscan::{Analysis, ScanConfig, ScanError, analyze_path};
use rayon::prelude::*;
use serde_json::json;

const USAGE: &str = "\
usage:
  audioscan [--compact] [--strict] [--threshold <dB>] [--min-gap <s>] <file>
  audioscan batch <dir> [--out <file.jsonl>] [--jobs auto|<N>] [--strict] [--threshold <dB>] [--min-gap <s>]";

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
            "-h" | "--help" => {
                println!("{USAGE}");
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

fn cmd_batch(args: &[String]) -> ExitCode {
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
            "-h" | "--help" => {
                println!("{USAGE}");
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

    let scan = || -> Vec<(PathBuf, Result<Analysis, ScanError>)> {
        files
            .par_iter()
            .map(|p| {
                let path_str = p.to_string_lossy().to_string();
                // Isolate each file: a panic in one decode becomes an error row,
                // never an aborted batch.
                let result = catch_unwind(AssertUnwindSafe(|| analyze_path(&path_str, &config)))
                    .unwrap_or_else(|_| {
                        Err(ScanError::Decode("panicked while decoding".to_string()))
                    });
                (p.clone(), result)
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
    results.sort_by(|a, b| a.0.cmp(&b.0));

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

    let (mut ok, mut failed) = (0u64, 0u64);
    for (path, res) in &results {
        let path_str = path.to_string_lossy();
        let line = match res {
            Ok(analysis) => {
                ok += 1;
                serde_json::to_string(analysis).unwrap_or_else(|e| {
                    json!({"schema_version": 1, "path": path_str, "error": format!("serialize: {e}")})
                        .to_string()
                })
            }
            Err(e) => {
                failed += 1;
                json!({"schema_version": 1, "path": path_str, "error": e.to_string()}).to_string()
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
        "audioscan: scanned {} file(s): {ok} ok, {failed} failed",
        results.len()
    );
    ExitCode::SUCCESS
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
            if p.is_dir() {
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
