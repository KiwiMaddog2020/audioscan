# audioscan

Decode an audio file once and report its format, EBU R128 loudness, and silence
windows as JSON. One fast native pass instead of two or three `ffmpeg` shellouts.

## Why

`endenza-bootleg` measures loudness and finds silence by spawning `ffmpeg` and
scraping its stderr text (see `loudness.py`, `silencedetect.py`). That fully
decodes the file once per measurement and is fragile: it already cost a real bug,
reading ffmpeg's first per-frame `I: -70` line instead of the Summary block.
`audioscan` decodes each file a single time with `symphonia`, measures loudness
with the real `ebur128` library, finds silence in the same pass, and prints
structured JSON. Same numbers, fewer decodes, nothing to regex.

## Build

```bash
cargo build --release      # binary at target/release/audioscan
```

## Use

```bash
audioscan [--compact] [--threshold <dB>] [--min-gap <s>] <file>
```

- `--compact` one-line JSON (default is pretty)
- `--threshold` silence threshold in dB (default -30, Bootleg's tuned value)
- `--min-gap` shortest silence to report, in seconds (default 5.0, Bootleg's value)

## Output

```json
{
  "path": "take.wav",
  "container": "wav",
  "codec": "pcm_s16le",
  "sample_rate": 48000,
  "channels": 2,
  "bits_per_sample": 16,
  "duration_sec": 212.5,
  "integrated_lufs": -14.2,
  "loudness_range_lu": 8.6,
  "true_peak_dbtp": -1.1,
  "silence_threshold_db": -30.0,
  "silence_min_gap_sec": 5.0,
  "silences": [[6.0, 12.0]]
}
```

Loudness fields are `null` when a file is too short or quiet to measure.
`silences` uses the same `[start, end]` seconds convention Bootleg's
`segments_from_silences` already consumes.

## Validation

Checked against ffmpeg's `ebur128` filter (Bootleg's current ground truth) on
generated signals:

| signal | metric | audioscan | ffmpeg |
|---|---|---|---|
| 1 kHz @ -3 dBFS + 6 s silence | integrated | -6.26 LUFS | -6.3 LUFS |
| | true peak | -3.0 dBTP | (-3 dBFS sine) |
| | silence | [6.0, 12.0] | (built at 6-12 s) |
| varied -6/-18/-3/-14/-9 dBFS | integrated | -9.46 LUFS | -9.5 LUFS |
| | loudness range | 11.0 LU | 11.0 LU |

Reproduce:

```bash
python3 tools/make_signal.py samples/signal.wav
cargo run -- samples/signal.wav
ffmpeg -hide_banner -nostats -i samples/signal.wav -af ebur128 -f null -
```

Note: LRA only agrees on signals with real loudness variation. On a degenerate
two-level signal the percentile gating is unstable in both tools and they
disagree, which is expected, not a bug.

## Formats

Enabled: wav, flac, mp3, aac/m4a, ogg/vorbis, adpcm, mkv (symphonia defaults plus
`mp3`, `aac`, `isomp4`). Not yet enabled: aiff, alac, opus. Add the feature in
`Cargo.toml` when a recording needs it.

## Status and next steps

Standalone by design, intentionally not wired into Bootleg. Bootleg's V2 pipeline
is a locked design (`~/.claude/orchestrator/docs/BOOTLEG_DESIGN_2026-05-25.md`);
swapping its ffmpeg-shell loudness/silence paths for an `audioscan` subprocess is
a separate, gated change. Because the contract is "run a binary, read JSON," it
fits Bootleg's "JSON adapters only, never Python imports" boundary cleanly.

Candidate directions:
- parallel batch mode (`rayon`) for the ~100 GB Zoom-archive import
- a C ABI so `veranota` (Swift) can call the same core
- bump `symphonia` to 0.6
