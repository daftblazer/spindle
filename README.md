# Spindle

Spindle is a GNOME app for authoring Blu-ray discs with interactive menus, in
the spirit of DVDStyler. You import videos, lay out menus by dragging buttons, text
and thumbnails around a canvas, link buttons to titles, chapters or other
menus, and build a BDMV folder that plays in VLC and on standalone players.

Built with Rust, GTK 4 and libadwaita.

## Features

- Drag-and-drop menu editor with snapping guides, resize handles, undo/redo,
  and multi-selection (shift/ctrl-click or rubber band)
- Align and distribute tools, copy/cut/paste between menus, keyboard nudging
- **Templates** (Ctrl+T): generate a complete, linked set of menus in one
  of five themes:
  - *TV Show*: main menu with **Play All** and **Episodes**, and paginated
    episode pages with thumbnails and Previous/Next/Main Menu
  - *TV Show (List)*: the same, with a text episode list (names and running
    times) instead of thumbnails
  - *Movie*: full-screen still, Play Movie, Scene Selection, and Extras
  - *Simple List*: a single list of titles with Play All
- **Play All** buttons play every title in order, then return to the menu
- **Remote preview** (F5): try the menu with the arrow keys and Enter, as a
  player would, including following links to other menus
- Buttons with labels and video-frame thumbnails; normal, selected and
  activated states with frame, text-color or underline highlights
- Still menus, and motion menus with a looping video background and music
- A frame picker (scrubber, frame stepping, filmstrip) to choose each
  episode's thumbnail, used everywhere the episode appears, and the frame of
  any thumbnail button or video still
- **Subtitles**: use the subtitle streams embedded in each video (and
  matching sidecar files like `Movie.en.srt`), choose which to keep, set
  their language, mark forced tracks and pick the one shown by default.
  - Text subtitles (SRT, ASS/SSA, WebVTT, MP4 text) are rendered with libass;
    ASS keeps its fonts, colours, positioning and effects, other formats use
    a configurable style with a live preview
  - PGS subtitles (embedded or `.sup`) are decoded and re-encoded, rescaled
    when the disc resolution differs
- Chapters, placed at the playhead or generated at a fixed interval, plus
  one-click chapter selection menus
- Automatic remote-control navigation between buttons, with per-button
  overrides
- Drop files straight from the file manager onto the canvas or the media bin
- Native HDMV output, with no BD-J and no Java:
  - Interactive Graphics menus
  - MovieObject navigation commands
  - playlists with chapter marks
  - clip info with EP maps
  - BDAV `.m2ts` streams from Spindle's own muxer

## How the disc is built

| Stage | What happens |
| --- | --- |
| Render | Menus are drawn with cairo/pango, the same code as the editor canvas |
| Encode | `ffmpeg` produces BD-compliant H.264 (High@4.1, x264 `bluray-compat`) and AC-3 or LPCM |
| Subtitles | `src/subtitles` renders or decodes every track into timed bitmaps and encodes PGS display sets |
| Mux | `src/bluray/ts` re-schedules the PES packets into a 192-byte-packet BDAV stream, inserting PCR/PAT/PMT/SIT and the IG menu stream and recording the EP map |
| Navigate | `src/bluray/nav` writes `index.bdmv`, `MovieObject.bdmv`, `*.mpls` and `*.clpi` |

Buttons can't call `PlayPL` in HDMV, so every title gets its own movie object.
A button that plays a title stores the chapter in GPR0 and runs `JumpTitle`.

## Building

Flatpak (recommended; bundles FFmpeg with x264):

```sh
flatpak install org.gnome.Sdk//50 org.freedesktop.Sdk.Extension.rust-stable  # pick the branch matching the SDK
flatpak-builder --user --install --force-clean build-dir io.github.daftblazer.Spindle.json
flatpak run io.github.daftblazer.Spindle
```

Native (needs GTK ≥ 4.18, libadwaita ≥ 1.7, libass ≥ 0.15, and
`ffmpeg`/`ffprobe` with libx264):

```sh
meson setup _build --prefix=$PWD/_install
ninja -C _build install
./_install/bin/spindle
```

To build a disc without the GUI:

```sh
spindle --new-project show.spindle episodes/*.mkv     # titles from media files
spindle --apply-template show 0 show.spindle          # show|show-list|movie|list, theme 0-4
spindle --build show.spindle OUTPUT_DIR
```

Set `SPINDLE_FFMPEG` or `SPINDLE_FFPROBE` to use specific binaries.

## Testing

`cargo test` covers the format writers (HDMV command encodings, EP map
coarse/fine split, RLE, PSI CRCs), the project model and menu navigation.

The built discs have been checked with libbluray's `index_dump`, `mobj_dump`,
`mpls_dump` and `clpi_dump`. They have also been driven through libbluray's
HDMV engine, which showed the menu and navigated between buttons, titles,
chapters and menus correctly. To play a result with menus:

```sh
vlc bluray:///path/to/OUTPUT_DIR
```

## Status

This is an early preview. Not implemented yet:

- ISO (UDF 2.50) output and burning; use a burning tool on the BDMV folder
- DVD (VobSub) and DVB bitmap subtitles, and multiple audio tracks
- Menu templates
