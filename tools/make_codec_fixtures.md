# Codec fixtures

The codec fixtures are checked in under `tests/fixtures/` so the test suite
does not need ffmpeg or LAME at runtime.

Exact commands used:

```sh
mkdir -p tests/fixtures
ffmpeg -f lavfi -i sine=frequency=1000:duration=1.000:sample_rate=48000 -ac 1 -c:a pcm_s16le /tmp/audioscan_codec_tone.wav
ffmpeg -i /tmp/audioscan_codec_tone.wav -c:a flac tests/fixtures/tone_1khz_1s.flac
lame -b 128 /tmp/audioscan_codec_tone.wav tests/fixtures/tone_1khz_1s_gapless.mp3
```

The transient WAV is a 1.000 s mono 1 kHz tone at 48 kHz. The MP3 is encoded
with LAME so the file carries encoder delay and padding metadata for the
gapless-duration regression test.
