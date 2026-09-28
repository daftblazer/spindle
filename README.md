# Spindle

Spindle is a GNOME app for authoring Blu-ray discs with interactive menus, in
the spirit of DVDStyler. You import videos, design menus by dragging buttons,
text and pictures around a canvas (or start from a template), link buttons to
titles, chapters and other menus, and build a disc image, a BDMV folder, or
burn a BD-R/BD-RE directly.

Built with Rust, GTK 4 and libadwaita. Discs use native HDMV navigation and
menus (no BD-J, no Java), written by Spindle's own muxer and format writers.

## Features

### Menus

- Drag-and-drop editor with snapping guides, a snap grid, zoom and pan,
  resize handles, undo/redo and multi-selection
- Align, distribute, group, lock, hide and reorder items; copy and paste
  between menus; duplicate whole menus
- Text with outlines, glow, shadows, letter and line spacing; shapes with
  gradients, borders and rounded corners or ellipses; images with opacity,
  rounded corners, cropping and drop shadows (transparent PNG logos work)
- Buttons with video-frame thumbnails or your own artwork for each state,
  and frame, text, underline, filled or arrow highlights; copy one button's
  look to every button
- 16:9 menus, and 4:3 menus that fill the screen on 4:3 discs
- Still menus, motion menus (a looping video background with a chosen start
  point), menu music, intro videos that play first, fades, and timeouts that
  play a title or show another menu
- **Pop-up menus** shown over the movie with the remote's Pop-up key:
  chapters, audio and subtitle choices, and links back to the menus
- **Language setup**: presets such as "English" (English audio, signs and
  songs subtitles) or "Japanese" (Japanese audio, full English subtitles),
  matched to each episode's own tracks, with a Setup menu to choose them
- Automatic remote-control navigation, shown on the canvas; Alt+drag from
  one button to another to set a direction by hand
- **Remote preview** (F5): try menus with the arrow keys and Enter, as a
  player would

### Templates

Ctrl+T opens a gallery in three categories, each style in any of eleven
colour palettes, with optional **Season** and **Disc** lines:

- **TV Show**: Classic, Classic List, Showcase, Showcase List, Minimal,
  Streaming, Split and Broadcast, each with a main menu (Play All, Episodes, Setup) and
  paginated episode pages
- **Movie**: Classic, Showcase and Minimal, with scene selection and extras
- **Collection**: a simple list of every title

Templates also create the Setup menu and pop-up menus, and can use an
imported logo instead of the title text. **Regenerate Episode Menus**
makes the episode pages again (after adding episodes, or in another style)
while keeping every other menu and relinking the buttons that led to them.

Your own designs can be saved as **custom templates** (Save Menus as
Template…), used on other shows, and exported as a single
`.spindle-template` file to share. See
[docs/custom-templates.md](docs/custom-templates.md) for the format.

### Titles

- Chapters placed at the playhead or generated at an interval, and
  paginated chapter selection menus in the disc's theme
- A frame picker to choose each title's thumbnail
- **Multiple audio tracks** from the video or from separate files (with a
  delay adjustment). Audio that Blu-ray players decode is copied without
  re-encoding: AC-3, DTS, DTS-HD Master Audio, Dolby TrueHD (with the AC-3
  core players need added) and Dolby Digital Plus. Other audio becomes AC-3
  or 16/24-bit LPCM, mixed to mono, stereo or 5.1 as chosen, and optionally
  evened out to −23 LUFS (EBU R128)
- **Picture controls** per title: deinterlacing or inverse telecine,
  crop (with black bar detection), aspect ratio override, and fitting the
  whole picture, filling the screen or stretching; HDR video is tone mapped
  to standard range
- **Subtitles** from the video or sidecar files (`Movie.en.srt`): SRT, ASS/SSA,
  WebVTT and MP4 text are rendered with libass (ASS keeps its styling), PGS is
  re-encoded; choose tracks, languages, forced and default tracks
- **Keep Original Video**: put compatible H.264 on the disc without
  re-encoding, after a compatibility check
- Previews: encode a short clip exactly as it will be on the disc and play it

### Building

- Output as an **ISO image** (UDF 2.50), a BDMV folder, or **burned** straight
  to a BD-R or BD-RE, then read back and verified; ISOs can also be burned
  later (Burn Disc Image…)
- Checks before building (missing files, dead ends, buttons outside the safe
  area, disc size…), each with a link to fix it, and **Fit to Disc** to pick
  the bitrate that fills a BD-25, BD-50 or BD-100
- Every Blu-ray video format: 1080p 23.976/24, 1080i 25/29.97 (interlaced
  video kept as it is, film with 3:2 pulldown), 720p 23.976/24/50/59.94 and
  SD 576i/480i in 16:9 or 4:3 (for 4:3 televisions)
- Quality presets: Fast, Balanced, and Best (x264 two-pass for the best
  picture and accurate sizes), with the build time estimated beforehand
- Titles are encoded several at a time on computers with many cores, and
  the build shows each step with its progress, speed and time left
- **Hardware encoding** (VA-API for AMD/Intel, NVENC for NVIDIA) for quick
  test discs; it is faster but noticeably lower quality, so final discs
  should use Software (x264)
- Encodes are reused when rebuilding, so changing a menu doesn't encode the
  videos again
- Autosave with crash recovery, and projects that keep finding their media
  when the project folder is moved

## How the disc is built

| Stage | What happens |
| --- | --- |
| Render | Menus are drawn with cairo/pango, the same code as the editor canvas |
| Encode | `ffmpeg` produces Blu-ray compliant H.264 (High@4.1, x264 `bluray-compat`, or constrained VA-API/NVENC) and AC-3 or LPCM; `src/media/picture.rs` builds the deinterlacing, cropping, scaling and tone mapping filters |
| Subtitles | `src/subtitles` renders or decodes every track into timed bitmaps and encodes PGS display sets |
| Menus | `src/bluray/ig` encodes Interactive Graphics: pages, buttons, palettes, fade effects and HDMV button commands |
| Mux | `src/bluray/ts` schedules the PES packets into a 192-byte-packet BDAV stream with PCR/PAT/PMT/SIT, menu and subtitle streams, and records the EP map; copied HD audio is split into core and extension packets as Blu-ray requires |
| Navigate | `src/bluray/nav` writes `index.bdmv`, `MovieObject.bdmv`, `*.mpls` and `*.clpi` |
| Image | `src/bluray/udf.rs` writes a UDF 2.50 image with a metadata partition, as BD-ROM uses |
| Burn | `xorriso` writes the image; `src/burn.rs` reads the disc back with SCSI commands and compares it |

Buttons can't call `PlayPL` in HDMV, so every title has its own movie object:
a button stores the chapter in GPR0 and runs `JumpTitle`. Play All, the
language setting and menu intros keep their state in further GPRs.

## Building Spindle

Flatpak (recommended; bundles FFmpeg with x264, zimg, VA-API and NVENC,
libass and xorriso):

```sh
flatpak install org.gnome.Sdk//50 org.freedesktop.Sdk.Extension.rust-stable//25.08
flatpak-builder --user --install --force-clean build-dir io.github.daftblazer.Spindle.json
flatpak run io.github.daftblazer.Spindle
```

GNOME Builder can build and run the same manifest.

Native (needs GTK ≥ 4.18, libadwaita ≥ 1.7, libass ≥ 0.15, `ffmpeg`/`ffprobe`
with libx264 and zimg, and `xorriso` for burning):

```sh
meson setup _build --prefix=$PWD/_install
ninja -C _build install
./_install/bin/spindle
```

The fonts used by the templates (Cantarell, Montserrat, Lato, TeX Gyre Heros
Condensed) come with the GNOME Flatpak runtime; native builds need them
installed for menus to look as designed.

## Command line

Discs can be made without the GUI:

```sh
spindle --new-project show.spindle episodes/*.mkv            # titles from media files
spindle --apply-template showcase - show.spindle "Season 2" "Disc 1"
spindle --build show.spindle "Show Season 2.iso"             # or a folder name for BDMV
spindle --burn "Show Season 2.iso" /dev/sr0                  # burn and verify
```

Template styles: `show`, `show-list`, `showcase`, `minimal`, `streaming`,
`split`, `broadcast`, `movie`, `movie-showcase`, `movie-minimal`, `list`. The
theme is a palette number from 0 to 10, or `-` for the style's own.

Custom templates: `--save-template PROJECT OUT.spindle-template NAME
[DESCRIPTION]`, `--install-template FILE`, and a `.spindle-template` file (or
a saved template's name) in place of the style for `--apply-template`.

Other commands: `--preview PROJECT TITLE START SECONDS [SUBTITLE]`,
`--check-video FILE`, `--make-image FOLDER IMAGE [LABEL]`,
`--render-menus PROJECT DIR` (every menu as a PNG) and `--disc-status DRIVE`.

Set `SPINDLE_FFMPEG`, `SPINDLE_FFPROBE` or `SPINDLE_XORRISO` to use specific
binaries, and `SPINDLE_JOBS` to choose how many titles are encoded at once.

## Testing

`cargo test` covers the format writers (HDMV commands, EP maps, IG and PGS
segments, RLE, PSI CRCs, UDF structures), the project model, validation,
templates (every style checked for overlaps, the safe area and reachability),
language matching, encoder arguments and burning helpers. GitHub Actions
runs clippy and the tests on Fedora 44.

Built discs have been checked with libbluray's dump tools and driven through
its HDMV engine: menus, pop-up menus, chapters, language choices, intros and
timeouts, from folders and ISO images. ISO images have also been checked
with `udfinfo` and 7-Zip. To play a disc with its menus:

```sh
vlc bluray:///path/to/disc.iso
```

## Status

Spindle is in active development. Discs play correctly in VLC (libbluray);
testing on standalone players and consoles, and burning real discs, is under
way. Not supported yet: DVD (VobSub) and DVB subtitles, UHD/HDR and 3D
discs, and BD-J menus.

## License

GPL-3.0-or-later. See [COPYING](COPYING).
