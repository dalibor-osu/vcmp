#!/bin/sh
# Generates the placeholder app icon: a "V" in Sora, Catppuccin Pink on the dark
# theme's base colour. VCMP has no logo yet; replace these files when it does.
# Needs ImageMagick 7 (`magick`). Run from anywhere; the files land next to this script.
set -eu

here=$(cd "$(dirname "$0")" && pwd)
font="$here/../../assets/fonts/sora/Sora[wght].ttf"

magick -size 1024x1024 xc:none \
    -fill '#1e1e2e' -draw 'roundrectangle 64,64 959,959 200,200' \
    -font "$font" -pointsize 640 -fill '#f5c2e7' \
    -gravity center -annotate +0+20 'V' \
    "$here/icon-1024.png"

for size in 32 128 256 512; do
    magick "$here/icon-1024.png" -resize "${size}x${size}" "$here/icon-$size.png"
done
magick "$here/icon-1024.png" -define icon:auto-resize=256,128,64,48,32,16 "$here/icon.ico"
rm "$here/icon-1024.png"
