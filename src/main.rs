//! audioscan CLI: a thin wrapper over the `audioscan` library.
//!
//!     audioscan [--compact] [--threshold <dB>] [--min-gap <s>] <file>
//!
//! All analysis lives in `src/lib.rs` (`audioscan::analyze_path`). This file
//! only parses arguments, formats JSON, and maps errors to exit codes.

use std::process::ExitCode;

use audioscan::{ScanConfig, analyze_path};

const USAGE: &str =
    "usage: audioscan [--compact] [--strict] [--threshold <dB>] [--min-gap <s>] <file>";

struct CliArgs {
    path: String,
    config: ScanConfig,
    pretty: bool,
}

enum Parsed {
    Run(CliArgs),
    Help,
    Version,
}

fn parse_args() -> Result<Parsed, String> {
    let mut path: Option<String> = None;
    let mut config = ScanConfig::default();
    let mut pretty = true;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--compact" => pretty = false,
            "--pretty" => pretty = true,
            "--strict" => config.strict = true,
            "--threshold" => {
                let v = args.next().ok_or("--threshold needs a value (dB)")?;
                config.threshold_db = v
                    .parse()
                    .map_err(|_| "--threshold must be a number".to_string())?;
            }
            "--min-gap" => {
                let v = args.next().ok_or("--min-gap needs a value (seconds)")?;
                config.min_gap_sec = v
                    .parse()
                    .map_err(|_| "--min-gap must be a number".to_string())?;
            }
            "-h" | "--help" => return Ok(Parsed::Help),
            "-V" | "--version" => return Ok(Parsed::Version),
            flag if flag.starts_with('-') => return Err(format!("unknown flag: {flag}")),
            file => {
                if path.is_some() {
                    return Err("only one input file is supported".to_string());
                }
                path = Some(file.to_string());
            }
        }
    }

    // Validate ranges here too, so a bad flag is a clean usage error (exit 2)
    // rather than surfacing from the library as a generic failure.
    config.validate().map_err(|e| e.to_string())?;
    let path = path.ok_or("no input file (usage: audioscan <file>)")?;
    Ok(Parsed::Run(CliArgs {
        path,
        config,
        pretty,
    }))
}

fn main() -> ExitCode {
    let cli = match parse_args() {
        Ok(Parsed::Run(cli)) => cli,
        Ok(Parsed::Help) => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Ok(Parsed::Version) => {
            println!("audioscan {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        Err(msg) => {
            eprintln!("audioscan: {msg}\n{USAGE}");
            return ExitCode::from(2);
        }
    };

    match analyze_path(&cli.path, &cli.config) {
        Ok(analysis) => {
            let json = if cli.pretty {
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
