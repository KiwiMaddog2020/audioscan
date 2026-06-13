---
title: "audioscan: one decode pass instead of three ffmpeg shellouts"
date: 2026-06-12
---

# audioscan: one decode pass instead of three ffmpeg shellouts

<p class="dek">What writing my first real Rust tool taught me about audio, FFI, and trusting your own numbers.</p>

<p class="meta">Kevin Madson · June 2026 · 5 min read</p>

> **If someone forwarded this to you:** I build and operate agentic systems on
> Claude Code, and audioscan is a small native tool that fell out of one of
> them. It decodes an audio file once and reports its format, EBU R128
> loudness, and silence windows as JSON. It is also the first real thing I
> wrote in Rust.

<p class="contact-card">
<a href="https://github.com/KiwiMaddog2020/audioscan">github.com/KiwiMaddog2020/audioscan</a>
<span class="sep">·</span>
<a href="mailto:kevinmadson@protonmail.com">kevinmadson@protonmail.com</a> <!-- pragma: allowlist -->
</p>

---

## The problem: scraping ffmpeg's stderr

One of my projects is a private catalog for my band's recordings. It needs two
boring things from every take: how loud it is, and where the silent gaps are, so
it can split a long rehearsal tape into songs.

The first version got both by shelling out to ffmpeg and scraping its stderr
text, once for loudness and once for silence detection. That has two problems.
It fully decodes the file once per measurement, so a single take is decoded two
or three times. And reading numbers out of a tool's diagnostic text is fragile
in a way that bites. It already had: the scraper read ffmpeg's first per-frame
`I: -70` line instead of the integrated value in the Summary block at the end of
the run. A per-frame reading near the start of a track is meaningless, so the
catalog was occasionally storing a loudness that was off by tens of decibels.
The fix in the scraper worked, but it was a patch on a method that was always
going to find a new way to fail.

## The fix: decode once, measure everything

audioscan decodes each file a single time with
[symphonia](https://github.com/pdeljanov/Symphonia), measures loudness with the
real [ebur128](https://crates.io/crates/ebur128) library (the same algorithm
ffmpeg's filter wraps), finds silence in that same pass, and prints structured
JSON. Same numbers, fewer decodes, nothing to regex.

```json
{
  "container": "wav", "codec": "pcm_s16le",
  "sample_rate": 48000, "channels": 2, "duration_sec": 212.5,
  "integrated_lufs": -14.2, "loudness_range_lu": 8.6, "true_peak_dbtp": -1.1,
  "silence_threshold_db": -30.0, "silences": [[6.0, 12.0]],
  "status": "ok", "warnings": []
}
```

The single pass is the whole point. As packets decode, the same samples flow
into the loudness meter and into a running window that tracks when the signal
drops below a threshold. One read of the file produces both answers, and the
JSON contract means the catalog calls a binary and reads stdout instead of
importing anything or parsing prose.

## Trusting the numbers

You cannot claim "same numbers" without checking, so I did, against ffmpeg's own
`ebur128` filter on generated signals:

| signal | metric | audioscan | ffmpeg |
| --- | --- | --- | --- |
| 1 kHz at -3 dBFS, 6 s of silence | integrated | -6.26 LUFS | -6.3 LUFS |
| | silence window | [6.0, 12.0] | built at 6 to 12 s |
| varied -6/-18/-3/-14/-9 dBFS | integrated | -9.46 LUFS | -9.5 LUFS |
| | loudness range | 11.0 LU | 11.0 LU |

The whole table reproduces from the repo with three commands. And one honest
caveat made it into the README rather than getting hidden: loudness range only
agrees when the signal has real variation. On a degenerate two-level test tone
the percentile gating that loudness range depends on is unstable in both tools,
and they disagree. That is expected, not a bug, so the README says exactly that.
A validation table that only shows the rows that agree is not a validation
table.

## What writing my first real Rust tool taught me

I came to this from Python, JavaScript, and Swift, so a few things were new.

**The C ABI was the surprising part.** audioscan builds as a library, a static
library, and a C dynamic library, with a [cbindgen](https://github.com/mozilla/cbindgen)-generated
header exposing three functions: analyze a path to a JSON string, free that
string, and report the version. The point is that a Swift app can call the same
analysis core directly, with no subprocess. The gotcha cost me an afternoon:
Rust's 2024 edition spells the export `#[unsafe(no_mangle)]`, and cbindgen 0.27
did not recognize that form, so it generated a header with no prototypes for the
`extern "C"` functions at all. The fix was a one-line bump to cbindgen 0.29 in
the build dependencies. I only caught it because the generated header was empty
where it should have had three functions, which is the kind of failure that
passes every test until someone tries to link against it.

**Formats are a feature flag, not a rewrite.** symphonia's defaults already
cover wav, flac, ogg/vorbis, and PCM; I added mp3, aac, and isomp4 for the lossy
files the catalog ingests. Adding aiff or opus later is a line in `Cargo.toml`,
not a new decoder.

**Batch mode is where the operational care lives.** Pointed at a directory, it
scans recursively and emits one JSON line per file, in parallel via
[rayon](https://crates.io/crates/rayon). Three decisions matter there. Each file
is isolated with panic capture, so one corrupt recording in a run of two
thousand becomes an error row instead of aborting the batch. The JSON Lines on
stdout are byte-identical no matter how many workers run, so per-file timing and
progress live on stderr and never pollute the data stream. And a cooperative
soft timeout, checked between packets, lets a wedged file stop at a deadline
instead of hanging the whole run, while legitimately long recordings are never
truncated unless you ask for a limit.

## What it is, and isn't

About a thousand lines of Rust, 27 integration tests across eight files (golden
loudness against ffmpeg, real codec fixtures, batch isolation, the C FFI, the
timeout, and malformed-input robustness), dual MIT and Apache licensed.

It is standalone on purpose. It is not yet wired into the catalog it was built
for, because swapping a production pipeline's measurement path is a separate,
gated change, not something to slip in next to a new tool. The "run a binary,
read JSON" contract is exactly what makes that swap clean when I make it. This is
one operator's tool for one job, done carefully, not a general media framework.

## The checkable numbers

Clone it, run `cargo build --release`, point the binary at a wav, and diff the
JSON against `ffmpeg -af ebur128`. The validation table, the codec fixtures, and
the batch behavior are all in the repository, and the numbers above are the ones
it actually prints. If they do not reproduce, that is a bug worth telling me
about.
