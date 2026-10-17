#!/bin/sh
# Regenerates the playback test fixtures. Needs ffmpeg with libmp3lame and libvorbis.
#
# The reference signal is one second of 44.1 kHz stereo: a 440 Hz sine on the left
# and a 660 Hz sine on the right, both at half amplitude. It is cut at frame 22433,
# mid-waveform, into tone-a (frames 0..22433) and tone-b (frames 22433..44100), and
# each part is encoded on its own in every format. Played back to back, a gapless
# player reproduces the uninterrupted sines; any gap or overlap shows up as a phase
# jump. The constants in src/local/tests.rs must match the numbers here.
set -eu
cd "$(dirname "$0")"

signal='aevalsrc=0.5*sin(2*PI*440*t)|0.5*sin(2*PI*660*t):s=44100:d=1'
split=22433

part() {
    name=$1
    trim=$2
    shift 2
    ffmpeg -nostdin -hide_banner -loglevel error -y -f lavfi -i "$signal" \
        -af "atrim=$trim,asetpts=PTS-STARTPTS" -map_metadata -1 -fflags +bitexact "$@" "$name"
}

for p in a b; do
    if [ "$p" = a ]; then trim="end_sample=$split"; else trim="start_sample=$split"; fi
    part "tone-$p.wav" "$trim" -c:a pcm_s16le
    part "tone-$p.flac" "$trim" -c:a flac
    part "tone-$p.mp3" "$trim" -c:a libmp3lame -b:a 192k
    part "tone-$p.ogg" "$trim" -c:a libvorbis -q:a 5
done
