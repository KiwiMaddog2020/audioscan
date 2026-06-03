#!/usr/bin/env python3
"""Generate a known test signal for validating audioscan.

Pure stdlib (no numpy). Writes 18 s of 48 kHz mono 16-bit PCM:

    [0-1s]   silence
    [1-6s]   1 kHz tone @ -3 dBFS
    [6-12s]  silence        <- 6 s, the only window that clears the 5 s min-gap
    [12-17s] 1 kHz tone @ -3 dBFS
    [17-18s] silence

So audioscan should report exactly one silence window, ~[6.0, 12.0], and a
loudness figure that matches whatever ffmpeg's ebur128 filter reports on the
same file.

    python3 tools/make_signal.py [output_path]
"""
import math
import pathlib
import struct
import sys
import wave

SR = 48_000


def tone(freq_hz: float, secs: float, dbfs: float) -> list[int]:
    amp = 10.0 ** (dbfs / 20.0)
    full = 32767.0
    n = int(SR * secs)
    return [
        int(max(-32768, min(32767, round(amp * full * math.sin(2 * math.pi * freq_hz * i / SR)))))
        for i in range(n)
    ]


def silence(secs: float) -> list[int]:
    return [0] * int(SR * secs)


samples = silence(1) + tone(1000, 5, -3) + silence(6) + tone(1000, 5, -3) + silence(1)

out = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else "samples/signal.wav")
out.parent.mkdir(parents=True, exist_ok=True)
with wave.open(str(out), "wb") as w:
    w.setnchannels(1)
    w.setsampwidth(2)
    w.setframerate(SR)
    w.writeframes(struct.pack("<%dh" % len(samples), *samples))

print(f"wrote {out}: 18 s, 1 kHz @ -3 dBFS, 6 s silence at [6.0, 12.0]")
