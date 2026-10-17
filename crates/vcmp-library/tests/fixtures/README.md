# Audio fixtures

One-second silent files, generated with FFmpeg 8 and committed so tests need no
encoder. They carry no tags; tests copy them to a temporary directory and write the
tags each case needs.

```bash
SRC="-f lavfi -i anullsrc=r=8000:cl=mono -t 1 -map_metadata -1 -fflags +bitexact -flags:a +bitexact"
ffmpeg $SRC -c:a libmp3lame -b:a 8k silence.mp3
ffmpeg $SRC -c:a flac silence.flac
ffmpeg $SRC -c:a libvorbis -q:a 0 silence.ogg
ffmpeg $SRC -c:a pcm_s16le silence.wav
ffmpeg -f lavfi -i anullsrc=r=48000:cl=mono -t 1 -map_metadata -1 -fflags +bitexact -c:a libopus -b:a 6k opus-in-ogg.ogg
```

`opus-in-ogg.ogg` is an Opus stream with an `.ogg` extension. v1 plays Ogg Vorbis
only, so the scanner must skip it.
