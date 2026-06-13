---
title: "Audioscan: one decode pass instead of three ffmpeg shellouts"
date: 2026-06-12
---

# Audioscan: one decode pass instead of three ffmpeg shellouts

<p class="dek">What writing my first real Rust tool taught me about audio, calling code across languages, and trusting your own numbers.</p>

<p class="meta">Kevin Madson · June 2026 · 5 min read</p>

> **If someone forwarded this to you:** I mix and master music, and I build
> software with AI agents on the side. Audioscan is a small, fast program that
> fell out of one of those projects. It reads an audio file once and reports its
> format, its loudness, and where the silent gaps are, as clean structured data.
> It is also the first real thing I have written in Rust, a language built for
> exactly this kind of fast, careful systems work.

<p class="contact-card">
<a href="https://github.com/KiwiMaddog2020/audioscan">github.com/KiwiMaddog2020/audioscan</a>
<span class="sep">·</span>
<a href="mailto:kevinmadson@protonmail.com">kevinmadson@protonmail.com</a> <!-- pragma: allowlist -->
</p>

---

## The problem: reading numbers out of another tool's status text

I mix and master music, and one of my projects is a private catalog of the
recordings I work on. It needs two boring things from every take: how loud it is
(so tracks sit at a consistent volume instead of jumping around), and where the
silent gaps are (so it can split a long recording into separate songs).

The first version got both by running ffmpeg, the standard audio command-line
tool, and reading the numbers out of the status text it prints while it works.
That has two problems. To get a number, the file has to be fully unpacked from its
compressed form into raw sound, and doing it this way unpacked each take two or
three times over. Worse, scraping numbers out of a tool's chatter is fragile in a
way that bites: my scraper once grabbed an early, meaningless reading from the
start of a track instead of the real summary printed at the end, so the catalog was
occasionally storing a loudness off by tens of decibels. I patched the scraper, but
it was a patch on a method that was always going to find a new way to fail.

## The fix: unpack once, measure everything

Audioscan unpacks each file a single time (using a Rust audio library called
[symphonia](https://github.com/pdeljanov/Symphonia)), measures loudness with the
real [ebur128](https://crates.io/crates/ebur128) library (the same math ffmpeg
uses, built to the EBU R128 standard that streaming services use to keep volume
consistent), finds the silent gaps in that same pass, and prints clean structured
data. Same numbers, fewer passes, nothing to scrape:

```json
{
  "container": "wav", "codec": "pcm_s16le",
  "sample_rate": 48000, "channels": 2, "duration_sec": 212.5,
  "integrated_lufs": -14.2, "loudness_range_lu": 8.6, "true_peak_dbtp": -1.1,
  "silence_threshold_db": -30.0, "silences": [[6.0, 12.0]],
  "status": "ok", "warnings": []
}
```

The single pass is the whole point. As the file is unpacked, the same stream of
sound feeds the loudness meter and a running check for when it drops to near
silence. One read of the file produces both answers, and because the output is
clean data, the catalog just runs the program and reads its output instead of
parsing prose.

## Trusting the numbers

You cannot claim "same numbers" without checking, so I did, against ffmpeg's own
loudness measurement on test signals:

| signal | metric | Audioscan | ffmpeg |
| --- | --- | --- | --- |
| 1 kHz tone, then 6 s of silence | loudness | -6.26 LUFS | -6.3 LUFS |
| | silence window | [6.0, 12.0] | built at 6 to 12 s |
| five tones at varied levels | loudness | -9.46 LUFS | -9.5 LUFS |
| | loudness range | 11.0 LU | 11.0 LU |

The whole table reproduces from the repo with three commands. And one honest
caveat made it into the README rather than getting buried: the loudness-range
number only agrees when the signal has real variation. On a flat two-level test
tone, the statistics that number depends on are shaky in both tools, and they
disagree. That is expected, not a bug, so the README says exactly that. A
validation table that only shows the rows that agree is not a validation table.

## What writing my first real Rust tool taught me

I came to this from Python, JavaScript, and Swift, so a few things were new.

**Letting other languages call it was the surprising part.** Audioscan can be
used not just as a standalone program but as a library that code in other
languages can call directly, without launching it as a separate process, which is
how I will eventually wire it into a Mac app. Setting that up cost me an afternoon
to one nasty gotcha: a version mismatch between Rust and the tool that generates
the bridge for other languages silently produced an empty bridge, with none of the
functions in it. A one-line version bump fixed it. I only caught it because the
generated file was blank where it should have listed three functions, which is the
kind of failure that passes every test right up until someone tries to use the
thing.

**New formats are a one-line change, not a rewrite.** The audio library already
handles the common formats; I added a few more (mp3, aac, and the kind of audio
inside mp4 files) for the compressed files the catalog takes in. Adding another
later is one line in a config file, not a new decoder.

**Running it over a whole folder is where the careful work lives.** Pointed at a
folder, it scans everything and prints one line of data per file, working on many
files at once for speed. Three decisions mattered. Each file is walled off, so one
corrupt recording in a run of two thousand becomes a single error row instead of
crashing the whole run. The data output stays identical no matter how many files
run in parallel, so progress messages go to a separate channel and never mix into
the data. And a gentle timeout, checked between chunks, lets a stuck file give up
at a deadline instead of hanging the entire run, while genuinely long recordings
are never cut off unless you ask for a limit.

## What it is, and isn't

About a thousand lines of Rust, 27 tests across eight files (loudness checked
against ffmpeg, real audio-file fixtures, the folder-scan isolation, the
other-language bridge, the timeout, and bad-input handling), released under two
permissive licenses.

It is standalone on purpose. It is not yet wired into the catalog it was built for,
because swapping out a production system's measurement path is a separate, careful
change, not something to slip in beside a brand-new tool. The "run a program, read
its data" design is exactly what will make that swap clean when I do it. This is one
person's tool for one job, done carefully, not a general-purpose media framework.

## The checkable numbers

Clone it, build it, point it at a `.wav` file, and compare its output to ffmpeg's
own loudness measurement. The validation table, the test audio files, and the
folder-scan behavior are all in the repository, and the numbers above are the ones
it actually prints. If they do not reproduce, that is a bug worth telling me about.

---

<p class="byline"><em>I build agentic systems across multiple coding LLMs. More of my research notes are <a href="/">here</a>.</em></p>
