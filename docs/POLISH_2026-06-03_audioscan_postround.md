# Polish re-rate — audioscan, Rounds 1-2 (post-round)

*Generated 2026-06-03 via the Polish protocol, Claude x Codex duet. Baseline: `POLISH_2026-06-03_audioscan.md`.*

## Aggregate trajectory

| | Pre (audit) | Post-R1 | Post-R2 |
|---|---:|---:|---:|
| Craft | ~5.7 | ~7.2 | **~7.8** |
| Fit | ~4.7 | ~6.4 | **~7.0** |

- **R1** = foundation + cheap correctness: library extraction, typed errors, JSON fields, multichannel silence fix, input validation (Claude) + golden test suite (Codex).
- **R2** = contract + failure semantics: `--strict`, `warnings[]`, truncation detection (Claude) + full-contract snapshot and strict/truncation tests (Codex).

## Categories moved

| Category | Craft (audit to now) | Fit | What landed |
|---|---|---|---|
| Tests & validation | 2 to 9 | 2 to 9 | 10 tests across `cli_golden.rs` (6) + `contract.rs` (4); full 17-field contract snapshot locks drift |
| Library / API readiness | 3 to 7 | 2 to 7 | `src/lib.rs`: `analyze_path` / `ScanConfig` / `Analysis` / `ScanError`; thin CLI (R1) |
| JSON contract durability | 4 to 9 | 3 to 8 | `schema_version`, `status`, `skipped_packets`, `warnings` + snapshot test (R1+R2) |
| Corrupt / partial reporting | 4 to 8 | 3 to 8 | `status:"partial"` + `warnings[]` + truncation detection + `--strict` (R2); no more silent partial success |
| Multichannel silence | 3 to 8 | 2 to 8 | per-frame power across channels; anti-phase regression-tested (R1) |
| Error typing | 4 to 8 | 3 to 7 | `thiserror` `ScanError`; `anyhow` dropped (R1) |
| Numeric input validation | 5 to 8 | 4 to 8 | NaN/Inf/negative rejected with exit 2, tested (R1) |
| Panic / adversarial safety | 5 to 7 | 4 to 6 | bounds-safe indexing + truncated-file no-panic test (R1) |
| Observability | 3 to 6 | 2 to 6 | `warnings[]` + `skipped_packets` + `status` surface failure modes (R2); elapsed/bytes still missing |
| True-peak hygiene | 6 to 7 | 6 to 7 | finite-filtered before serialization (R1) |

## Still open (R3)

- Batch mode + JSONL envelope + per-file status (the ~100 GB archive): craft 3 / fit 2, unchanged.
- Channel-layout loudness mapping (5.1/7.1): craft 4 / fit 3.
- Gapless timing for mp3/aac: craft 5 / fit 4.
- Observability elapsed/bytes: partial.
- ebur128 per-block history memory: documented, deferred.

## Commits (local, not pushed)

R1: `51e74c3` plan · `5a6eae3` lib+fixes · `7fc4099` golden tests · `2001507` re-rate
R2: `4d06202` --strict/warnings/truncation · `e35bfa8` contract+strict tests · this re-rate update
