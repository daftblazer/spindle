# Custom menu templates

A custom template is a main menu and an episode page layout, saved as one
`.spindle-template` file (JSON, with its pictures inside). Applied to a
show, it fills in the show's title, season, episode names, running times and
stills, and makes as many episode pages as the show needs. It can also hold
the designs of the Setup menu (the language choices) and the pop-up menus;
without them, Spindle makes those in the template's colours.

## Making one

* **In Spindle**: design the main menu and an episode page (starting from a
  built-in template is easiest), then choose **Save Menus as Template…** in
  the main menu. Save it to *My Templates*, or export it to a file to share.
  The project's Setup and pop-up menus are saved with it, as they look.
* **From the command line**:

  ```sh
  spindle --save-template show.spindle "Night Sky.spindle-template" "Night Sky" "A description"
  spindle --install-template "Night Sky.spindle-template"   # add it to My Templates
  spindle --apply-template "Night Sky.spindle-template" - other.spindle "Season 2" "Disc 1"
  ```

Templates in *My Templates* live in `~/.local/share/spindle/templates`
(inside the Flatpak: `~/.var/app/io.github.daftblazer.Spindle/data/spindle/templates`).
Open them from **Menu Templates → My Templates**, where they can also be
imported, exported and removed, and pick them in **Regenerate Episode
Menus** to rebuild just the episode pages.

## The file

```json
{
  "format": "spindle-template",
  "version": 1,
  "name": "Night Sky",
  "description": "…",
  "author": "…",
  "palette": { "top": …, "bottom": …, "panel": …, "text": …, "dim": …, "selected": …, "activated": … },
  "fonts": { "heading": "Montserrat", "body": "Montserrat" },
  "shape": "Wide",
  "main": { "name": "Main Menu", "background": …, "items": [ … ], "default_button": "<item id>" },
  "episodes": {
    "page": { "name": "Episodes", "background": …, "items": [ … ] },
    "slot": [ … ],
    "columns": 1, "rows": 8, "step": [0, 80], "order": "columns"
  },
  "setup": { "page": { … }, "slot": [ … ], "columns": 1, "rows": 6, "step": [0, 100], "center": true },
  "popup": {
    "menu": { "name": "Pop-up Menu", "items": [ … ] },
    "chapters": { "page": { … }, "slot": [ … ], "columns": 12, "rows": 1, "step": [112, 0] },
    "audio": { "page": { … }, "slot": [ … ], "columns": 3, "rows": 1, "step": [420, 0] }
  },
  "images": { "<id>": { "name": "logo.png", "data": "<base64>" } }
}
```

Colours are `{ "r": 0.1, "g": 0.2, "b": 0.3, "a": 1.0 }` (0 to 1). The palette
and fonts are for the Setup and pop-up menus Spindle makes when the template
has none of its own; menus in the template keep exactly the colours and
fonts of their items.

Items are Spindle menu items as in a `.spindle` project (text, button,
image, shape), on a 1920 × 1080 canvas, with two optional fields:

* `"link"` on a button: what it does. `play_all`, `episodes` (the first
  episode page), `setup` (left out when the disc has no Setup menu),
  `main_menu`, `first_episode`, and on episode pages `episode` (the slot's
  episode), `previous` and `next` (left out on the first / last page). The
  Setup and pop-up menus have a few more, described below.
* `"still"` on a picture (or a button, for its thumbnail): which episode's
  frame it shows. `"first"`, `"episode"` (the slot's), `"page"` (the first on
  the page) or `{ "nth": 3 }` (the fourth on the page; on the main menu, of
  the show). Pictures of other files use an id from `images`.

### Episode pages

`page` holds everything that isn't an episode: backgrounds, the heading and
the Previous / Next / Main Menu buttons. `slot` holds the items of the first
episode (its button, a still, its running time…). The slot is repeated
`columns × rows` times per page, `step` apart (`[across, down]`), filling
rows first (`"order": "rows"`) or columns first (`"columns"`). A layout
without Previous / Next buttons gets them when a show needs several pages.

### Setup menu

`setup` (optional) is the menu of language choices, shown when the disc has
two or more. It is a layout like the episode pages: `page` holds everything
but the choices, and `slot` holds one choice, a button with
`"link": "language"` and `{name}` in its label ("English", "Japanese ·
English Subtitles"). The slot is repeated for each choice, `step` apart, and
the disc's default language is selected first. Buttons on the page can link
to `main_menu`.

The menu isn't paginated: `columns × rows` is the most choices the design
holds, and a disc with more gets the Setup menu Spindle makes itself. With
`"center": true`, fewer choices than that stay in the middle of the space
the full list takes (two choices in a panel made for six sit in its middle),
rather than at its start.

### Pop-up menus

`popup` (optional) holds the menus shown over a title with the remote's
Pop-up key. Their backgrounds are see-through, whatever the file says: draw
the bar or panel with shapes and pictures.

* `menu` is the pop-up itself, an ordinary menu. Its buttons link to
  `chapters` and `audio` (its two pages, below), `episodes`, `main_menu` and
  `setup`. A link with nowhere to go is left out (`chapters` when no title
  has any, `audio` and `setup` with one language), and the linked buttons
  left in its row close up around the row's middle (in a column, towards its
  top). Only the buttons move, so keep decorations off them.
* `chapters` (optional) is a layout with a slot for each chapter: a button
  with `"link": "chapter"` and `{number}` in its label. Chapters past
  `columns × rows` are left out.
* `audio` (optional) is a layout with a slot for each language choice, like
  `setup`. A disc with more choices than it holds gets the page Spindle
  makes itself.

On both pages a button with `"link": "back"` returns to the pop-up menu, and
`center` works as it does for `setup`. A page that is left out is made by
Spindle in the palette.

Templates can be applied without pop-up menus: turn off **Pop-up Menus** in
the template gallery, or add `--no-popups` on the command line.

### 16:9 or 4:3

A template is designed for one shape of screen. `"shape"` is `"Wide"` (16:9,
the default) or `"Standard"` (4:3). A 4:3 design uses the middle 1440 × 1080
of the canvas (x from 240 to 1680), which fills the screen of a 4:3 SD disc;
its safe area is 5% in from those edges. For a show on both kinds of disc,
make two templates ("Night Sky" and "Night Sky (4:3)").

Either can be applied to any disc: a 16:9 template on a 4:3 disc is
letterboxed, and a 4:3 template on a 16:9 disc sits in the middle of the
screen. The template gallery says when a template was made for the other
shape.

In Spindle, a menu's **Shape** (in its properties) is 16:9 or 4:3; the
canvas shades the sides of 4:3 menus. Saving 4:3 menus makes a 4:3 template.

Older templates could hold a 4:3 design beside the 16:9 one, in
`"standard"`. Spindle reads such a file as two templates, the 4:3 one with
" (4:3)" after its name, and keeps them as two files in *My Templates*.

### Placeholders

Text and button labels can hold:

| Placeholder | Becomes | Where |
| --- | --- | --- |
| `{title}` | The show's name (a logo replaces the first text holding it) | anywhere |
| `{season}`, `{disc}` | "Season 2", "Disc 1" | anywhere |
| `{edition}` | "Season 2 · Disc 1" | anywhere |
| `{count}` | "12 Episodes" | anywhere |
| `{page}`, `{pages}`, `{page_label}` | 1, 3, "Page 1 of 3" (empty on a single page) | episode pages |
| `{number}`, `{number2}` | 7, 07 | slot |
| `{name}` | The episode's name, without "S01E07 -" | slot |
| `{title_name}` | The name, or "Title 7" when it has none | slot |
| `{duration}`, `{minutes}` | 23:40, "24 min" | slot |
| `{name}` | The language choice | Setup and pop-up audio slot |
| `{number}` | The chapter's number | pop-up chapters slot |

In capitals (`{TITLE}`, `{EDITION}`…) they give the value in capitals. Text
that ends up empty is left out, and separators (`·`, `|`, `•`, `/`) left
hanging by an empty value are removed.
