# Custom menu templates

A custom template is a main menu and an episode page layout, saved as one
`.spindle-template` file (JSON, with its pictures inside). Applied to a
show, it fills in the show's title, season, episode names, running times and
stills, makes as many episode pages as the show needs, and adds Setup and
pop-up menus in its colours.

## Making one

* **In Spindle**: design the main menu and an episode page (starting from a
  built-in template is easiest), then choose **Save Menus as Template…** in
  the main menu. Save it to *My Templates*, or export it to a file to share.
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
  "main": { "name": "Main Menu", "background": …, "items": [ … ], "default_button": "<item id>" },
  "episodes": {
    "page": { "name": "Episodes", "background": …, "items": [ … ] },
    "slot": [ … ],
    "columns": 1, "rows": 8, "step": [0, 80], "order": "columns"
  },
  "standard": { "main": { … }, "episodes": { … } },
  "images": { "<id>": { "name": "logo.png", "data": "<base64>" } }
}
```

Colours are `{ "r": 0.1, "g": 0.2, "b": 0.3, "a": 1.0 }` (0 to 1). The palette
and fonts are for the Setup and pop-up menus; the main menu and episode
pages keep exactly the colours and fonts of their items.

Items are Spindle menu items as in a `.spindle` project (text, button,
image, shape), on a 1920 × 1080 canvas, with two optional fields:

* `"link"` on a button: what it does. `play_all`, `episodes` (the first
  episode page), `setup` (left out when the disc has no Setup menu),
  `main_menu`, `first_episode`, and on episode pages `episode` (the slot's
  episode), `previous` and `next` (left out on the first / last page).
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

### 4:3 menus

`standard` (optional) holds the same menus designed for 4:3 screens, used
when the disc is one of the 4:3 SD formats. A 4:3 design uses the middle
1440 × 1080 of the canvas (x from 240 to 1680), which fills a 4:3 screen;
its safe area is 5% in from those edges. Without `standard`, 4:3 discs get
the 16:9 design letterboxed.

In Spindle, a menu's **Shape** (in its properties) is 16:9 or 4:3; the
canvas shades the sides of 4:3 menus. Saving 4:3 menus as a template under
the name of one with only 16:9 menus (or the other way round) puts both in
one template.

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

In capitals (`{TITLE}`, `{EDITION}`…) they give the value in capitals. Text
that ends up empty is left out, and separators (`·`, `|`, `•`, `/`) left
hanging by an empty value are removed.
