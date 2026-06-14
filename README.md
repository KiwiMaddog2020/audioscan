# audioscan

Decode an audio file once and report its format, EBU R128 loudness, and silence
windows as JSON. One fast native pass instead of two or three `ffmpeg` shellouts.

## Why

I mix and master music, and a private catalog of mine needs two boring things
from every recording: how loud it is (so tracks sit at a consistent volume) and
where the silent gaps are (so it can split a long recording into songs). The
first version got both by running `ffmpeg`, the standard audio command-line tool,
and reading the numbers out of its status text. That fully decodes the file once
per measurement and is fragile: it already cost a real bug, reading ffmpeg's first
per-frame `I: -70` line instead of the final Summary block, storing a loudness off
by tens of decibels. `audioscan` decodes each file a single time with `symphonia`,
measures loudness with the real `ebur128` library (the same math ffmpeg uses, on
the EBU R128 standard that streaming services use to keep volume consistent), finds
silence in the same pass, and prints structured JSON. Same numbers, fewer decodes,
nothing to scrape.

## Install

```bash
cargo install audioscan
```

That installs the `audioscan` binary from crates.io. To install the latest from
source without cloning, use `cargo install --git https://github.com/KiwiMaddog2020/audioscan`.
Prebuilt macOS and Linux binaries are attached to each
[release](https://github.com/KiwiMaddog2020/audioscan/releases).

## Build

```bash
cargo build --release      # binary at target/release/audioscan
```

## Use

```bash
audioscan [--compact|--pretty] [--strict] [--timeout <s>] [--threshold <RMS-dBFS>] [--min-gap <s>] <file>
```

- `--pretty` pretty-printed JSON (default)
- `--compact` one-line JSON
- `--strict` fail instead of returning `status: "partial"` when decode is incomplete
- `--timeout` per-file soft decode deadline in seconds (default: none / unbounded)
- `--threshold` silence threshold in RMS dBFS (default -30)
- `--min-gap` shortest silence to report, in seconds (default 5.0)

`--timeout <secs>` bounds how long a single file may spend decoding. It is a
cooperative soft deadline checked between packets, so a slow or wedged file
stops at the limit instead of running unbounded. A timed-out file is reported as
`status: "partial"` with a `decode exceeded timeout of <N>s` warning, or, under
`--strict`, an error. The default is no timeout, so legitimately long recordings
are never truncated unless you set one. In batch mode the deadline applies per
file and the batch continues past a timed-out file.

On success, single-file mode prints `audioscan: analyzed <path> in <N.NN>s`
to stderr, so the JSON on stdout stays clean and pipeable.

### Batch

```bash
audioscan batch <dir> [--out <file.jsonl>] [--jobs auto|<N>] [--strict] [--timeout <s>] [--threshold <RMS-dBFS>] [--min-gap <s>]
```

Batch mode recursively scans known audio extensions under `<dir>` and emits
compact JSON Lines, one row per file. Without `--out`, rows are written to
stdout. `--jobs auto` uses rayon's default worker count; `--jobs <N>` pins the
batch to a fixed positive worker count.

Each batch JSONL row, success or error, also includes `"bytes": <input file
size in bytes on disk>`, a deterministic per-row field for sorting or spotting
large inputs. Successful rows contain the analysis object shown below plus
`bytes`. Per-file failures are written as
`{"schema_version":1,"path":"...","error":"...","bytes":1234}`. `bytes` is a
batch-row-only operational field; the single-file output object below does not
include it. Each file is isolated with panic capture, so a panic or decode
failure for one recording becomes an error row instead of aborting the batch.

Batch mode prints a live per-file progress line to stderr as each file
completes, followed by the summary and slowest-file timing report:

```text
audioscan: [3/2000] /archive/take_03.wav (1182ms)
audioscan: scanned 2000 file(s): 1996 ok, 3 partial, 1 failed in 41.7s
audioscan: slowest: big.flac 3201ms (118.0 MB), long.wav 1980ms (90.2 MB), take_03.wav 1182ms (44.1 MB)
```

Because the breadcrumb streams as files finish, not just at the end, a wedged or
slow file is visible live as the file with no completion line yet, and the run
is not silent until the end.
The `slowest:` line lists the slowest files with each file's elapsed time in
milliseconds and size.
stdout JSON Lines stay byte-identical across `--jobs` counts, so per-file
wall-clock timing and progress live on stderr instead of in the JSONL stream.

Exit codes are `0` when the command completes and writes its requested output,
`1` for fatal runtime failures such as unreadable output paths, no discovered
audio files, or a failed single-file scan, and `2` for usage or invalid-config
errors. Batch per-file error rows do not by themselves make the batch command
fail once the JSONL output has been written.

## Output

```json
{
  "schema_version": 1,
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
  "silences": [[6.0, 12.0]],
  "status": "ok",
  "skipped_packets": 0,
  "warnings": []
}
```

`status` is `ok` for a clean decode and `partial` when the scan completed after
skipping corrupt packets, detecting an incomplete stream, or exceeding a
configured timeout. `warnings[]` holds human-readable diagnostics for partial
output; it is empty for clean output.
With `--strict`, partial decodes become errors instead of JSON analysis rows.
`container` is the lowercased file extension from the input path, or `""` for an
extensionless path.

`integrated_lufs` and `loudness_range_lu` are `null` together when the input is
too short or quiet to measure. `true_peak_dbtp` is `null` only for digital
silence, where there is no inter-sample peak to report. `silences` uses a simple
`[start, end]` seconds convention. Silence boundaries are quantized to the roughly 30 ms analysis window,
matching ffmpeg `silencedetect`.

## Validation

Checked against ffmpeg's `ebur128` filter on generated signals:

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

Standalone by design, intentionally not yet wired into the catalog it was built
for. Swapping a production pipeline's measurement path for an `audioscan`
subprocess is a separate, careful change. Because the contract is "run a binary,
read JSON," that swap stays clean when I make it.

Candidate directions:
- a C interface so a Swift app can call the same core directly, with no subprocess
- bump `symphonia` to 0.6
