# Polish goal-loop — audioscan, Rounds 4-7 (GOAL MET: craft + fit >= 9.0)

*Generated 2026-06-03 via the Polish protocol as a `/goal` loop: Claude x Codex duet + hyperdrive,
iterating audit -> build -> re-rate until both axes cleared 9.0. Continues
`POLISH_2026-06-03_audioscan_postround.md` (Rounds 1-3, baseline craft ~8.4).*

## Goal

> "Refine and get our final polish rating up to 9.0 ... duet protocol and hyperdrive all in one ...
> loop until you achieve these results, treat it as a /goal."

Bar: **craft >= 9.0 AND fit >= 9.0** on the two-axis rubric (craft = universal code quality,
fit = fit-to-purpose for a fast oracle over a ~100 GB mono/stereo band archive), with no regression.

## Aggregate trajectory

| | R3 (baseline) | R4 | R5 | R6 | R7 |
|---|---:|---:|---:|---:|---:|
| Craft | ~8.4 | 8.54 | 8.78 | 8.91 | **9.04** |
| Fit | ~8.0 | 8.77 | 8.75 | **9.04** | **9.18** |
| hit_nine | - | no | no | no (fit only) | **YES** |

Each round was re-rated by an independent panel of re-scorers plus one adversarial Tier-2 reviewer
over the round's own diff. Verdicts were instructed not to inflate; R4 and R5 honestly returned
sub-9 with concrete residuals, which drove the next round. **Zero regressions across all four rounds.**

## What each round landed

- **R4** correctness + contract truth-up: loudness-null contract (integrated + LRA null together when
  unmeasurable; true-peak null only on digital silence), mid-stream layout guard, `impl AsRef<Path>`,
  real-codec fixtures (FLAC + gapless MP3), package metadata + dual LICENSE + CI.
- **R5** API durability: `#[non_exhaustive] ScanError`, a `Status` enum with a `Deserialize`
  round-trip, `SCHEMA_VERSION` + value-golden tests, CI matrix + doctest + MSRV job, publish-exclude.
- **R6** the binding-fit fix (observability + contract): deterministic per-row `bytes` in batch JSONL
  via `#[serde(flatten)]`, a stderr `slowest:` timing report kept off byte-identical stdout,
  single-file stderr timing, `#[serde(deny_unknown_fields)]`, a literal `schema_version == 1` pin on a
  success row + exhaustive `Analysis {..}` destructure, named `MIN_DECODED_PERCENT` const, RMS-dBFS
  doc precision, Cargo `homepage`/`documentation`/`docs.rs` metadata. **Fit crossed 9.0 (9.04).**
- **R7** the binding-craft fix (watchdog + live progress): a `--timeout <secs>` cooperative per-packet
  decode deadline (timed-out file = `partial`, or `--strict` error; batch continues past it), a live
  `[k/total] <path> (<ms>)` stderr breadcrumb streamed as each file completes (a wedged file is the one
  with no completion line), a `cargo test --release` CI job, BatchRecord documented serialize-only.
  **Craft crossed 9.0 (9.04).**

## The binding-constraint chain (why the loop converged)

1. R5 verdict: the binding axis was **Observability** (7.6/7.8) -> R6 fixed it -> Observability ~9.2/9.4,
   fit reached 9.04.
2. R6 verdict: the binding craft constraint was **no per-file decode watchdog / live progress** (a
   wedged decode stalls a large batch silently) -> R7 fixed it -> Robustness 8.6/8.8 -> 9.1/9.2,
   Observability -> 9.2/9.4, craft reached 9.04.

Each verdict named the single highest-leverage item; each round closed exactly that item at the right
architectural location (the deadline lives inside `analyze_path` and composes with the existing
`catch_unwind` isolation, so a timeout is a non-fatal partial, not an aborted batch).

## Final category scores (R7 panel)

| Category | Craft | Fit |
|---|---:|---:|
| Observability + diagnostics | 9.2 | 9.4 |
| Robustness + failure semantics | 9.1 | 9.2 |
| API ergonomics / public surface | 9.3 | 9.4 |
| JSON-contract durability | 9.0 | 9.1 |
| Test coverage + CI | ~9.0 | ~9.0 |
| DSP correctness | 8.7 | 8.9 |

DSP correctness is the sole sub-9 anchor and the single biggest remaining craft lever.

## Deferred (honest residuals, none blocking the goal)

- **DSP golden-value cross-check** (DEFENSIBLE NET-NEW WORK, biggest lever): no tolerance test
  cross-checking integrated LUFS / LRA / true-peak against an independent reference (ffmpeg `ebur128`
  or libebur128 vectors). Deliberately deferred: this is a flag-don't-master triage oracle. Building it
  would carry DSP and the aggregate into the mid-9s and is the natural R8.
- **Single hung `decode()` call** (GENUINE PLATEAU / unavoidable): a cooperative between-packets
  deadline cannot interrupt one already-hung call; only a detached-thread hard kill that abandons the
  worker would, which safe Rust cannot do cleanly. The trade is documented (lib.rs, README) and the
  live breadcrumb already renders a wedged file visible (no completion line).
- **Sub-file heartbeat** for a single 40-minute file (minor defensible): breadcrumb is per-completion.
- **BatchRecord-must-not-deserialize footgun**: documented, not yet test-locked (cheap, low value).
- **`parse_f64_flag` accepts `inf`**: caught one layer deep by `validate()`'s `is_finite()` (verified
  `--timeout inf` exits 2); the `inf` case is just not asserted in `tests/timeout.rs` (one line).

## Verification (final state, HEAD `a01154a`)

- `cargo fmt --check` clean · `cargo clippy --all-targets -- -D warnings` clean
- **25 tests pass in debug AND release** (cli_golden 6, contract 5, batch 4, robustness 3, codecs 2,
  timeout 4 + 1 doctest) · adversarial Tier-2: zero regressions, every claim reproduced
- Live-verified: `--timeout 0.001` -> partial + timeout warning; `--timeout 60` -> clean ok; batch
  streams `[k/N]` breadcrumbs; stdout byte-identical SHA across `--jobs 1` vs `4`.

## Commits (9, local, not pushed)

R4: `903fcb8` src lane · `bd67156` codec fixtures · `12e5951` package/CI
R5: `8467962` non_exhaustive/Status/round-trip · `acdb4bf` CI matrix/doctest/MSRV/exclude
R6: `3c7de8f` telemetry/contract-harden/doc-precision · `eb2244e` crate metadata/docs/contract test
R7: `0f8661c` decode-timeout + live breadcrumb · `a01154a` release CI + timeout/progress docs

Baseline R1-R3 (craft ~8.4) was pushed earlier at `c14e082`; these 9 are the goal-loop, awaiting a
push decision.
