# Polish re-rate — audioscan, Rounds 1-3 (final)

*Generated 2026-06-03 via the Polish protocol, Claude x Codex duet. Baseline: `POLISH_2026-06-03_audioscan.md`.*

## Aggregate trajectory

| | Pre (audit) | Post-R1 | Post-R2 | Post-R3 |
|---|---:|---:|---:|---:|
| Craft | ~5.7 | ~7.2 | ~7.8 | **~8.4** |
| Fit | ~4.7 | ~6.4 | ~7.0 | **~8.0** |

- **R1** foundation + correctness: lib extraction, typed errors, JSON fields, multichannel silence fix, input validation + golden tests.
- **R2** contract + failure semantics: `--strict`, `warnings[]`, truncation detection + contract snapshot tests.
- **R3** scale: `batch` mode (recursive, rayon-parallel, JSONL, per-file panic isolation, deterministic, `--jobs`), gapless decoding + batch tests.

## Categories moved (audit -> final)

| Category | Craft | Fit | What landed |
|---|---|---|---|
| Tests & validation | 2 to 9 | 2 to 9 | 13 tests across `cli_golden.rs` (6), `contract.rs` (4), `batch.rs` (3); full contract snapshot |
| Batch / archive readiness | 3 to 8 | 2 to 8 | `audioscan batch <dir>`: recursive walk, rayon, JSONL `{...}`/`{path,error}`, panic-isolated, `--jobs`, path-sorted, `--out` (R3) |
| JSON contract durability | 4 to 9 | 3 to 8 | schema_version/status/skipped_packets/warnings + snapshot lock |
| Corrupt / partial reporting | 4 to 8 | 3 to 8 | status partial + warnings + truncation detection + `--strict`; batch isolates per file |
| Library / API readiness | 3 to 7 | 2 to 7 | `src/lib.rs` public API; thin CLI |
| Multichannel silence | 3 to 8 | 2 to 8 | per-frame power across channels; anti-phase regression-tested |
| Error typing | 4 to 8 | 3 to 7 | `thiserror` `ScanError`; `anyhow` dropped |
| Numeric input validation | 5 to 8 | 4 to 8 | NaN/Inf/negative rejected, tested |
| Gapless timing | 5 to 7 | 4 to 7 | `enable_gapless` for mp3/aac duration accuracy (R3) |
| Observability | 3 to 7 | 2 to 7 | warnings + skipped_packets + status + batch summary |
| Panic / adversarial safety | 5 to 7 | 4 to 7 | bounds-safe indexing + batch `catch_unwind` isolation |

## Deferred (honest "not worth 10/10 now")

- **Channel-layout surround loudness mapping** (craft 4 / fit 3): low value for mono/stereo band recordings; revisit only if surround material appears.
- **ebur128 per-block history memory**: documented; switch to `Mode::HISTOGRAM` only if very-long-file accuracy ever demands it.
- **Observability elapsed/bytes**: minor; add if batch telemetry is wanted.

## Commits (9, local, not pushed)

R1: `51e74c3` plan · `5a6eae3` lib+fixes · `7fc4099` golden tests · `2001507` re-rate
R2: `4d06202` strict/warnings/truncation · `e35bfa8` contract tests
R3: `e8400d9` batch+gapless · `bfccec0` batch tests · this re-rate
