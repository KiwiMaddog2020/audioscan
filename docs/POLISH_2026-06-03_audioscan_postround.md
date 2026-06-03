# Polish re-rate — audioscan, Round 1 (post-round)

*Generated 2026-06-03 via the Polish protocol, Round 1 of the Claude x Codex duet. Baseline: `POLISH_2026-06-03_audioscan.md`.*

## Aggregate deltas

| | Pre (audit) | Post-R1 | Delta |
|---|---:|---:|---:|
| Craft | ~5.7 | **~7.2** | +1.5 |
| Fit | ~4.7 | **~6.4** | +1.7 |

R1 was the foundation + cheap-correctness round: library extraction, typed errors, JSON contract fields, the multichannel silence fix, and input validation (Claude lane), plus the black-box golden test suite (Codex lane).

## Categories moved in R1

| Category | Craft pre to post | Fit pre to post | What landed |
|---|---|---|---|
| Tests & validation | 2 to 8 | 2 to 8 | 6 black-box golden tests (mono+silence, anti-phase guard, pure silence, short, truncated, bad-args); `cargo test` green |
| Library / API readiness | 3 to 7 | 2 to 7 | `src/lib.rs`: `analyze_path` / `ScanConfig` / `Analysis`; thin CLI |
| Error typing | 4 to 8 | 3 to 7 | `thiserror` `ScanError`; `anyhow` dropped |
| Multichannel silence | 3 to 8 | 2 to 8 | per-frame power across channels; anti-phase no longer false silence (regression-tested) |
| Numeric input validation | 5 to 8 | 4 to 8 | NaN/Inf/negative rejected with exit 2 (tested) |
| JSON contract durability | 4 to 6 | 3 to 6 | `schema_version` + `status` + `skipped_packets` (field-tested; full golden-JSON snapshot deferred to R2) |
| Panic / adversarial safety | 5 to 7 | 4 to 6 | bounds-safe sample indexing (truncated-file no-panic test); mid-stream spec-change still open |
| True-peak hygiene | 6 to 7 | 6 to 7 | finite-filtered before serialization |
| Observability | 3 to 4 | 2 to 4 | `status` + `skipped_packets` surfaced; elapsed / bytes still missing |

## Still open (R2 / R3)

- Batch mode + JSONL envelope + per-file status (R3): craft 3 / fit 2, unchanged.
- Channel-layout loudness mapping (R3): craft 4 / fit 3.
- Gapless timing for mp3/aac (R3): craft 5 / fit 4.
- Corrupt reporting: `status:"partial"` + `skipped_packets` landed, but `--strict` and the truncated-vs-clean-EOF distinction are deferred to R2.
- ebur128 per-block history memory: behavior documented, not changed (deferred).

## Commits (local, not pushed)

- `51e74c3` docs: polish audit + plan
- `5a6eae3` refactor: lib + R1 fixes (Claude lane)
- `7fc4099` test: golden suite (Codex lane)
- this re-rate doc
