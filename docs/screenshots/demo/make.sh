#!/bin/bash
# Makes the demo show ("Far Meridian", made up, with artwork drawn by art.py)
# and retakes the screenshots in docs/screenshots.
#
#   docs/screenshots/demo/make.sh          # everything
#   docs/screenshots/demo/make.sh media    # only the demo episodes and project
#
# Needs cargo, ffmpeg, ImageMagick, python3 with cairo, weston, bwrap and
# dbus-run-session (the fedora distrobox has them). Work files, the demo
# episodes included, go in $SPINDLE_DEMO_DIR (default ~/.cache/spindle-demo).
set -e
R=$(cd "$(dirname "$0")/../../.." && pwd)
HERE=$R/docs/screenshots/demo
W=${SPINDLE_DEMO_DIR:-$HOME/.cache/spindle-demo}
B=$W/target/debug/spindle
SHOW="Far Meridian"

# A debug build that loads its resources from the work folder: the generated
# src/config.rs names the install prefix, so it is swapped while building.
build() {
  mkdir -p "$W/res" "$W/schemas"
  glib-compile-resources --sourcedir="$R/src" --target="$W/res/spindle.gresource" "$R/src/spindle.gresource.xml"
  cp "$R"/data/*.gschema.xml "$W/schemas/" && glib-compile-schemas "$W/schemas"
  cp -p "$R/src/config.rs" "$W/config.rs.orig"
  trap 'cp -p "$W/config.rs.orig" "$R/src/config.rs"' EXIT
  sed -i "s|^pub static PKGDATADIR.*|pub static PKGDATADIR: \&str = \"$W/res\";|" "$R/src/config.rs"
  (cd "$R" && CARGO_TARGET_DIR="$W/target" cargo build)
  cp -p "$W/config.rs.orig" "$R/src/config.rs"
  trap - EXIT
}

# Six episodes of 22 to 25 minutes: ten seconds of each picture, repeated
# without encoding it again, with two silent audio tracks and subtitles.
media() {
  local d="$W/home/Videos/$SHOW" i=0
  rm -rf "$W/art" "$d" && mkdir -p "$W/art" "$d"
  python3 "$HERE/art.py" "$W/art"
  for f in "$W"/art/*.png; do magick "$f" -seed 1 -attenuate 0.10 +noise Gaussian "$f"; done
  printf '1\n00:00:02,000 --> 00:00:06,000\nThe road east begins where the maps end.\n\n2\n00:00:08,000 --> 00:00:12,000\nThen we had better start walking.\n' > "$W/art/subs.srt"
  while IFS='|' read -r name mins; do
    i=$((i + 1))
    ffmpeg -nostdin -v error -y -loop 1 -framerate 24000/1001 -i "$W/art/e$i.png" -frames:v 240 -vf scale=1920:1080,format=yuv420p \
      -c:v libx264 -preset fast -crf 14 -g 240 -bf 0 "$W/art/e$i.mkv"
    ffmpeg -nostdin -v error -y -stream_loop -1 -i "$W/art/e$i.mkv" -f lavfi -i anullsrc=r=48000:cl=stereo -f lavfi -i anullsrc=r=48000:cl=stereo -i "$W/art/subs.srt" \
      -t $((mins * 60 + 17)) -map 0:v -map 1:a -map 2:a -map 3:s -c:v copy -c:a ac3 -b:a 96k -c:s srt \
      -metadata:s:a:0 language=jpn -metadata:s:a:0 title=Japanese -metadata:s:a:1 language=eng -metadata:s:a:1 title=English -metadata:s:s:0 language=eng \
      "$d/$SHOW - 0$i - $name.mkv"
  done <<'LIST'
Departure|24
The Salt Road|23
Lantern Bay|24
Glasswater|22
A Quiet Summit|24
Homeward|25
LIST
  cp "$W/art/key.png" "$d/Key Art.png"
}

# project STYLE OUT.spindle: the demo project with a template applied.
project() {
  local d="$HOME/Videos/$SHOW"
  "$B" --new-project "$2" "$d"/*.mkv "$d/Key Art.png"
  python3 - "$2" "$SHOW" <<'PY'
import json, sys
p = json.load(open(sys.argv[1]))
p['disc']['name'] = sys.argv[2]
for t in p['titles']:
    t['name'] = t['name'].split(' - ')[-1]
json.dump(p, open(sys.argv[1], 'w'), indent=1)
PY
  "$B" --apply-template "$1" - "$2" "Season One" "Disc 1"
}

# shot NAME PROJECT [VAR=value...]: the window under a headless weston, at 2x.
shot() {
  local name=$1 proj=$2 sock=wl-$BASHPID
  shift 2
  (
    export XDG_RUNTIME_DIR=$(mktemp -d) GSETTINGS_SCHEMA_DIR="$W/schemas" GSETTINGS_BACKEND=memory
    weston --backend=headless --socket=$sock --width=1600 --height=1000 >/dev/null 2>&1 &
    weston=$!
    sleep 1.5
    env WAYLAND_DISPLAY=$sock GSK_RENDERER=cairo ADW_DEBUG_COLOR_SCHEME=prefer-dark SPINDLE_SCREENSHOT="$W/shots/$name.png" \
      SPINDLE_SCREENSHOT_SIZE=1440x900 SPINDLE_SCREENSHOT_SCALE=2 SPINDLE_SCREENSHOT_DELAY=8 "$@" \
      timeout 90 dbus-run-session -- "$B" "$proj" 2>&1 | grep -i "panicked\|screenshot failed" || true
    kill $weston 2>/dev/null
  )
}

shots() {
  local f="$HOME/Videos/$SHOW/$SHOW.spindle" s
  rm -rf "$W/shots" "$W/styles" && mkdir -p "$W/shots" "$W/styles"
  project split "$f"
  shot editor "$f" SPINDLE_SCREENSHOT_MENU=0 SPINDLE_SCREENSHOT_ITEM=12 &
  shot templates "$f" SPINDLE_SCREENSHOT_MENU=0 SPINDLE_SCREENSHOT_ACTION=win.templates &
  shot title "$f" SPINDLE_SCREENSHOT_TITLE=0 SPINDLE_SCREENSHOT_TAB=audio &
  wait
  shot build "$f" SPINDLE_SCREENSHOT_MENU=0 SPINDLE_SCREENSHOT_ACTION=win.build &
  shot popup "$f" SPINDLE_SCREENSHOT_MENU=3 SPINDLE_SCREENSHOT_ITEM=3 &
  wait
  # Menus as the disc shows them, in several styles.
  for s in show showcase minimal streaming split broadcast; do
    project $s "$W/styles/$s.spindle"
    "$B" --render-menus "$W/styles/$s.spindle" "$W/styles/$s" 2>/dev/null
  done
}

round() { # IN OUT RADIUS
  magick "$1" -alpha set \( +clone -alpha extract -fill black -colorize 100 -fill white -draw "roundrectangle 0,0,%[fx:w-1],%[fx:h-1],$3,$3" \) -compose CopyOpacity -composite "$2"
}

# Rounded corners and a soft shadow on a transparent ground, which suits
# light and dark pages.
frame() {
  local o=$R/docs/screenshots t=$W/tmp f m i=0
  mkdir -p "$t"
  for f in editor templates title build popup; do
    round "$W/shots/$f.png" "$t/$f.png" 26
    magick "$t/$f.png" \( +clone -background black -shadow 45x28+0+18 \) +swap -background none -layers merge +repage -resize 2000x -strip -define png:compression-level=9 "$o/$f.png"
  done
  for m in showcase/00-Main_Menu streaming/01-Episodes_1 broadcast/00-Main_Menu show/01-Episodes minimal/00-Main_Menu split/01-Episodes; do
    i=$((i + 1))
    magick "$W/styles/$m.png" -resize 640x360 "$t/m$i.png"
    round "$t/m$i.png" "$t/m$i.png" 14
  done
  magick montage "$t"/m[1-6].png -tile 3x2 -geometry +10+10 -background none -strip -define png:compression-level=9 "$o/menus.png"
}

# The rest runs with the work folder's home as /home/sam, so the window
# shows tidy paths.
if [ "$1" = --inside ]; then
  shots
  frame
  exit
fi
build
[ -d "$W/home/Videos/$SHOW" ] && [ "$1" != media ] || media
if [ "$1" = media ]; then
  HOME=$W/home project split "$W/home/Videos/$SHOW/$SHOW.spindle"
  echo "$W/home/Videos/$SHOW"
  exit
fi
exec bwrap --dev-bind / / --tmpfs /home --bind "$W" "$W" --bind "$W/home" /home/sam --setenv HOME /home/sam --setenv SPINDLE_DEMO_DIR "$W" "$0" --inside
