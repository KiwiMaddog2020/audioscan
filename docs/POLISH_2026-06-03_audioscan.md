# Polish — audioscan (Rust DSP CLI + Bootleg adapter)

*Generated 2026-06-03 via the Polish protocol (`plugins/endenza/skills/polish/`), run as a Claude × Codex duet (mutual-polish ensemble per `docs/DUET_PROTOCOL.md`).*

## 1. Subject + bar

- **Subject:** the `audioscan` Rust codebase (`~/Code/audioscan`, `src/main.rs` 372 lines) plus its Bootleg subprocess JSON contract.
- **Project:** audioscan — decode an audio file once via `symphonia`, emit EBU R128 loudness + silence + format probe as JSON; about to become a reusable library with a batch mode.
- **Bar:** Apple-quality / production-grade / ready to grow into a library.
- **Methodology:** two-axis (craft × fit) per category, two independent lanes (Claude + Codex), Opus-merged. Evidence is `file:line` in `src/main.rs` unless noted.

## 2. Duet provenance + score reconciliation

| Lane | Aggregate craft | Aggregate fit | Note |
|---|---:|---:|---|
| Claude | 6.8 | 6.5 | Over-graded; self-bias (authored the code this session) |
| Codex (independent) | 4.65 | 3.91 | Read symphonia + ebur128 source; weighted extraction blockers heavily |
| **Reconciled** | **≈ 5.7** | **≈ 4.7** | Codex's fit is better-anchored to the stated "reusable library" bar; the single-pass core is genuinely strong, but tests/lib/batch/contract are real blockers |

The reconciliation leans toward Codex on the fit axis: measured against "reusable library, batch-ready," the tool is a validated prototype, not a library. The single-pass decode architecture is the genuine craft high point (8/8).

## 3. Per-theme tables

Legend: **✓both** = both lanes caught it · **C** = Codex-only catch · **Cl** = Claude-only catch.

### DSP correctness
| # | Category | Craft | Fit | Gap | Notes (evidence) |
|---|---|---:|---:|---:|---|
| 1 | Single-pass decode architecture | 8 | 8 | 2 | One open + one packet loop feeds loudness and silence together (`analyze` loop). The objective met. ✓both |
| 2 | EBU R128 integration | 7 | 7 | 3 | `EbuR128::new(ch, sr, I\|LRA\|TRUE_PEAK)`, interleaved `add_frames_f32`. Correct, streams. ✓both |
| 3 | **Multichannel silence bug** | 3 | 2 | 8 | `SilenceTracker::push` folds channels by arithmetic mean (`acc / ch`). Anti-phase stereo `[+1,-1] → 0` reads as false silence. Use RMS/power across channels. **✓both** |
| 4 | Channel-layout loudness mapping | 4 | 3 | 7 | Only channel *count* is passed; ebur128 maps mechanically and marks channels >6 Unused. 5.1/7.1 mis-weighted. **C** |
| 5 | Gapless timing | 5 | 4 | 6 | `FormatOptions::enable_gapless` defaults false → MP3/AAC encoder delay/padding shifts duration + silence offsets from the true timeline. Bootleg ingests mp3. **C** |
| 6 | True-peak handling | 6 | 6 | 4 | Max over channels → dBTP; guarded for 0 → null, but NOT finite-filtered like integrated/LRA are. **C** (partial Cl) |
| 7 | Silence windowing + finalization | 7 | 6 | 4 | 30 ms windows; `finish()` does evaluate the final partial window. Fixed window is an undocumented, untested contract. ✓both |

### Robustness & failure semantics
| # | Category | Craft | Fit | Gap | Notes |
|---|---|---:|---:|---:|---|
| 8 | **Silent partial success** | 4 | 3 | 7 | Corrupt `DecodeError` skipped (unreported); truncated/IO break still returns success JSON from partial samples. No `status`/`partial`/`skipped` signal. Dangerous for a 100 GB batch. **C** (I had mis-scored this as a *positive*) |
| 9 | Panic / spec-change safety | 5 | 4 | 6 | Stale initial `ch` drives `samples[f*ch+c]` indexing; a mid-stream channel/rate change can panic. `sample_buf...unwrap()`. ✓both (C sharper on cause) |
| 10 | Numeric input validation | 5 | 4 | 6 | `--threshold` / `--min-gap` accept NaN/Inf/negative with no validation. ✓both |

### Library / API readiness
| # | Category | Craft | Fit | Gap | Notes |
|---|---|---:|---:|---:|---|
| 11 | **Library boundary** | 3 | 2 | 8 | No `src/lib.rs`; all types private in `main.rs`; `analyze` private; `process::exit` in parseable logic. Not consumable as a library, the core objective. ✓both |
| 12 | Error typing | 4 | 3 | 7 | `anyhow::Result` throughout. Wrong for a library boundary; callers can't match. Needs `thiserror` `ScanError`. ✓both |

### JSON contract
| # | Category | Craft | Fit | Gap | Notes |
|---|---|---:|---:|---:|---|
| 13 | **Contract durability** | 4 | 3 | 7 | `Analysis` has no `schema_version`/`status`/`warnings`; README shows an example but no versioned schema. Python subprocess contract needs versioning + golden tests + null policy. ✓both |

### Testing
| # | Category | Craft | Fit | Gap | Notes |
|---|---|---:|---:|---:|---|
| 14 | **Automated tests** | 2 | 2 | 8 | Zero in-repo Rust tests; `Cargo.toml` declares no dev-deps/test target; only fixture is a one-case mono WAV script. Validation was manual. The headline craft gap. ✓both |

### Performance / memory
| # | Category | Craft | Fit | Gap | Notes |
|---|---|---:|---:|---:|---|
| 15 | Memory scaling | 6 | 5 | 5 | SilenceTracker O(1), SampleBuffer reused — but without `Mode::HISTOGRAM`, ebur128 retains per-block energy histories for the whole programme. Not strictly O(1); grows with duration. **C** (corrects my "O(1)" claim) |
| 16 | Concurrency | 6 | 5 | 5 | Single-threaded; fine for one file, batch objective needs rayon. ✓both |

### CLI / UX / observability
| # | Category | Craft | Fit | Gap | Notes |
|---|---|---:|---:|---:|---|
| 17 | CLI ergonomics | 4 | 3 | 7 | Manual parse; `--help` exits inside parsing; one input only; no `--version`. ✓both |
| 18 | Observability / diagnostics | 3 | 2 | 8 | No elapsed time, byte count, decoder, skipped-packet count, or partial status. 100 GB scans need auditable signals. **C** |
| 19 | Batch / archive readiness | 3 | 2 | 8 | Config holds one `path`; parser rejects multiple files; no envelope/per-file status/ordering. ✓both |

### Docs / packaging
| # | Category | Craft | Fit | Gap | Notes |
|---|---|---:|---:|---:|---|
| 20 | Documentation | 7 | 6 | 4 | README strong on purpose/usage/schema/validation/formats; missing schema version, failure modes, channel + gapless policy. ✓both |
| 21 | Packaging / deps | 5 | 5 | 5 | symphonia 0.5 + curated features; no license/description/MSRV; aiff/alac/opus off. ✓both |

## 4. Top 10 highest-leverage gaps

| # | Gap | Categories | Cost | Round |
|---|---|---|---|---|
| 1 | Zero automated tests | 14 (+ all, as regression net) | Med | R2 |
| 2 | Not library-shaped (lib.rs + typed errors) | 11, 12 | Med | R1 |
| 3 | Multichannel silence mono-fold bug | 3 | Low-Med | R1 |
| 4 | Silent partial success on corrupt/truncated | 8, 18 | Med | R2 |
| 5 | JSON contract unversioned/unhardened | 13 | Low-Med | R1 |
| 6 | Numeric input validation + finite-filter true peak | 6, 10 | Low | R1 |
| 7 | Channel-layout loudness mapping | 4 | Med | R3 |
| 8 | Gapless timing drift (mp3/aac) | 5 | Low-Med | R3 |
| 9 | Panic on mid-stream spec change | 9 | Med | R2 |
| 10 | Batch envelope + observability | 18, 19 | Med | R3 |

## 5. Sequenced plan to 10/10 (routed Codex/Claude by shape)

- **Round 1 — foundation + cheap correctness (the v1 core, hardened):**
  - `[Claude]` Extract `src/lib.rs`: public `analyze_path` / `ScanConfig` / `ScanResult` / `ScanError` (thiserror); `main.rs` becomes a thin CLI; remove `process::exit` from parseable logic. (gaps 2, 12)
  - `[Codex]` Replace silence mono-average with a power/RMS-across-channels policy; classify silent only when all audible channels are below threshold. (gap 3)
  - `[Codex]` Validate numeric inputs (reject NaN/Inf/negative threshold+min-gap); finite-filter true peak. (gap 6)
  - `[Claude]` Add `schema_version` + `status` fields to the JSON (coupled with the lib types). (gap 5)
- **Round 2 — tests + failure semantics:**
  - `[Codex]` Golden fixture test suite: mono tone+silence, anti-phase stereo, multichannel, pure silence, short file, truncated/corrupt, extreme options; golden JSON snapshots; pin the ffmpeg loudness A/B (skip if absent). (gap 1)
  - `[Claude]` Explicit partial/corrupt reporting: count skipped packets, distinguish truncated vs clean EOF, `status: ok|partial|error`, `--strict`. (gaps 4, 9)
- **Round 3 — batch + harder DSP:**
  - `[Claude design + Codex plumbing]` Batch mode: JSONL envelope `{schema_version, results[], summary}` or per-line `{path, ...}`/`{path, error}`, `--jobs auto` (rayon), stable ordering, per-file status. (gap 10)
  - `[Claude/Codex + parity tests]` Channel-layout mapping (symphonia → ebur128) or warn on ambiguous >6ch. (gap 7)
  - `[Codex]` `enable_gapless` for lossy formats + mp3/aac duration tests vs ffmpeg. (gap 8)
- **One Tier-2 cross-model review per round** (the *other* engine reviews the integrated diff) per `DUET_PROTOCOL.md`.

## 6. Cost estimate

| Round | Lanes | Wall | Tokens | Human |
|---|---|---|---|---|
| R1 | 1 Claude + 2 Codex | ~30-50 min | ~40-70k | 10-15 min review |
| R2 | 1 Claude + 1 Codex | ~30-45 min | ~40-60k | 10-15 min |
| R3 | mixed | ~60-90 min | ~60-90k | 15-20 min |

To **fit ≥ 9.0**: Rounds 1-2 (lib + tests + failure semantics + contract). To **10/10**: + Round 3 + deferred items.

## 7. Open questions for human judgment

- **Gapless policy:** match Bootleg's expectation (does Bootleg want media-timeline or decoded-timeline timestamps)? Drives gap 8.
- **Multichannel silence policy:** all-channels-below vs any-channel-below threshold — depends on what "silence" means for band practice (probably all-channels).
- **symphonia 0.6 bump:** unverified API; do it during R1's lib extraction or defer?
- **License:** repo is private; add one only if it may go public.

## 8. Honesty checklist

- [x] Every craft grade cites file:line evidence (both lanes)
- [x] Every fit grade cites the objective it serves/fails
- [x] Zero "seems reasonable" / "looks fine" / "could be better" phrasing
- [x] At least one sub-10 weakness per theme
- [x] Calibrated to audioscan's stated "reusable library" bar, not generic best practice
- [x] No flattery without evidence (Claude self-bias explicitly corrected against Codex)
- [x] Plan has both low-cost wins (R1) and deferred hard items (R3 + channel layout)

## Deferred / not cost-effective now

- `Mode::HISTOGRAM` tradeoff: document current per-block memory behavior; only switch if very-long-file accuracy demands it.
- `clap` migration: do it when the API splits (R1+), not before.
- aiff/alac/opus: enable per actual archive inventory, not speculatively.
- Packaging metadata (license/description/MSRV): batch into R3 polish.
