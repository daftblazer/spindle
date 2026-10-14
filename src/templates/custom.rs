// SPDX-License-Identifier: GPL-3.0-or-later

//! Custom templates: a main menu and an episode page layout, and optionally
//! the Setup and pop-up menus, saved from a project (or written by hand) as
//! one `.spindle-template` JSON file, with its pictures inside, to use again
//! on other shows or share. A template is designed for one shape of
//! screen, 16:9 or 4:3.
//!
//! Items are Spindle menu items. Text can hold placeholders: `{title}`,
//! `{season}`, `{disc}`, `{edition}` ("Season 2 · Disc 1") and `{count}`
//! ("12 Episodes") anywhere; `{number}`, `{number2}` ("01"), `{name}`,
//! `{title_name}`, `{duration}` and `{minutes}` in an episode slot;
//! `{page}`, `{pages}` and `{page_label}` ("Page 1 of 2") on episode pages.
//! In capitals (`{TITLE}`) they give the value in capitals. Text that ends
//! up empty is left out. Buttons get what they do from
//! `link`, and pictures of episodes from `still`.

use super::*;
use anyhow::Context as _;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

pub const FORMAT: &str = "spindle-template";
pub const EXTENSION: &str = "spindle-template";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CustomTemplate {
    pub format: String,
    pub version: u32,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub author: String,
    /// Colours of the Setup and pop-up menus made with it.
    pub palette: Palette,
    pub fonts: Fonts,
    /// The shape of screen it is designed for: 16:9, or 4:3 (in the middle
    /// of the canvas).
    #[serde(default)]
    pub shape: MenuShape,
    pub main: TemplateMenu,
    pub episodes: EpisodeLayout,
    /// The Setup menu, with a slot for each language choice. Without it
    /// Spindle makes one in the palette.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub setup: Option<EpisodeLayout>,
    /// The pop-up menus. Without them Spindle makes them in the palette.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub popup: Option<PopupLayout>,
    /// Templates used to carry a 4:3 design beside the 16:9 one; such a
    /// file is read as two templates (see `split`).
    #[serde(default, skip_serializing)]
    standard: Option<Standard>,
    /// Pictures used by items and backgrounds, by the id they use.
    #[serde(default)]
    pub images: BTreeMap<Id, Image>,
}

/// The 4:3 design of an older template file.
#[derive(Debug, Clone, Deserialize, PartialEq)]
struct Standard {
    main: TemplateMenu,
    episodes: EpisodeLayout,
    #[serde(default)]
    setup: Option<EpisodeLayout>,
    #[serde(default)]
    popup: Option<PopupLayout>,
}

/// The menus shown over a title with the remote's Pop-up key.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PopupLayout {
    /// The pop-up itself, with buttons linked to `chapters`, `audio`,
    /// `episodes` and `main_menu`.
    pub menu: TemplateMenu,
    /// Its page of chapters, with a slot for each chapter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chapters: Option<EpisodeLayout>,
    /// Its page of language choices, with a slot for each.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio: Option<EpisodeLayout>,
}

impl CustomTemplate {
    /// The templates in a file: one, or the 16:9 and the 4:3 design of a
    /// file from when one template held both (the 4:3 one named "… (4:3)").
    fn split(mut self) -> Vec<CustomTemplate> {
        let Some(s) = self.standard.take() else { return vec![self] };
        let mut square = CustomTemplate {
            name: format!("{} (4:3)", self.name),
            shape: MenuShape::Standard,
            main: s.main,
            episodes: s.episodes,
            setup: s.setup,
            popup: s.popup,
            ..self.clone()
        };
        // Saved from 4:3 menus alone, they stood in for the 16:9 design.
        if square.main == self.main {
            square.name = self.name;
            return vec![square];
        }
        square.drop_unused_images();
        self.drop_unused_images();
        vec![self, square]
    }

    fn drop_unused_images(&mut self) {
        let mut used = std::collections::HashSet::new();
        let mut menu = |m: &TemplateMenu, slot: &[TemplateItem]| {
            used.extend(m.background.image);
            for ti in m.items.iter().chain(slot) {
                match &ti.item.kind {
                    ItemKind::Image(i) => used.extend([i.asset]),
                    ItemKind::Button(b) => used.extend([b.images.normal, b.images.selected, b.images.activated].into_iter().flatten()),
                    _ => {}
                }
            }
        };
        menu(&self.main, &[]);
        let lists = [Some(&self.episodes), self.setup.as_ref(), self.popup.as_ref().and_then(|p| p.chapters.as_ref()), self.popup.as_ref().and_then(|p| p.audio.as_ref())];
        for l in lists.into_iter().flatten() {
            menu(&l.page, &l.slot);
        }
        if let Some(p) = &self.popup {
            menu(&p.menu, &[]);
        }
        self.images.retain(|id, _| used.contains(id));
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct Palette {
    pub top: Rgba,
    pub bottom: Rgba,
    pub panel: Rgba,
    pub text: Rgba,
    pub dim: Rgba,
    pub selected: Rgba,
    pub activated: Rgba,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Fonts {
    pub heading: String,
    pub body: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Image {
    /// File name, for its type.
    pub name: String,
    /// The file, base64 encoded.
    pub data: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TemplateMenu {
    pub name: String,
    pub background: Background,
    pub items: Vec<TemplateItem>,
    /// Item id of the button selected first.
    #[serde(default)]
    pub default_button: Option<Id>,
    #[serde(default)]
    pub fade_in: f64,
    #[serde(default)]
    pub fade_out: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TemplateItem {
    #[serde(flatten)]
    pub item: MenuItem,
    /// What the button does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<Link>,
    /// Which episode a picture (or button thumbnail) shows a frame of.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub still: Option<Still>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Link {
    PlayAll,
    /// The first episode page.
    Episodes,
    Setup,
    MainMenu,
    /// Play the first episode.
    FirstEpisode,
    /// Play the slot's episode.
    Episode,
    Previous,
    Next,
    /// The pop-up's page of chapters.
    Chapters,
    /// The pop-up's page of language choices.
    Audio,
    /// From a pop-up page back to the pop-up menu.
    Back,
    /// Play the slot's chapter.
    Chapter,
    /// Choose the slot's language.
    Language,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Still {
    /// The first episode.
    First,
    /// The episode of the slot.
    Episode,
    /// The first episode on the page.
    Page,
    /// The n-th episode (from 0) on the page, or of the show on the main
    /// menu; left out when there are fewer.
    Nth(u32),
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Order {
    /// Left to right, then down.
    #[default]
    Rows,
    /// Top to bottom, then across.
    Columns,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EpisodeLayout {
    /// Everything on a page but the episodes: backgrounds, the heading and
    /// the Previous / Next / Main Menu buttons.
    pub page: TemplateMenu,
    /// The items of the first episode; the others are placed `step` apart.
    pub slot: Vec<TemplateItem>,
    pub columns: u32,
    pub rows: u32,
    /// Distance to the next column and the next row.
    pub step: [f64; 2],
    #[serde(default)]
    pub order: Order,
    /// In a list of chapters or languages shorter than `columns × rows`:
    /// keep the slots in the middle of the space the full list takes.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub center: bool,
}

impl EpisodeLayout {
    fn per_page(&self) -> usize {
        (self.columns.max(1) * self.rows.max(1)) as usize
    }

    /// Offset of slot `k` on a page.
    fn offset(&self, k: usize) -> (f64, f64) {
        let (cols, rows) = (self.columns.max(1) as usize, self.rows.max(1) as usize);
        let (c, r) = match self.order {
            Order::Rows => (k % cols, k / cols),
            Order::Columns => (k / rows, k % rows),
        };
        (c as f64 * self.step[0], r as f64 * self.step[1])
    }

    /// How far a list of `n` slots moves to stay in the middle of the
    /// space a full one takes.
    fn centering(&self, n: usize) -> (f64, f64) {
        if !self.center || n == 0 {
            return (0.0, 0.0);
        }
        let (cols, rows) = (self.columns.max(1) as usize, self.rows.max(1) as usize);
        let (used_cols, used_rows) = match self.order {
            Order::Rows => (n.min(cols), n.div_ceil(cols)),
            Order::Columns => (n.div_ceil(rows), n.min(rows)),
        };
        (cols.saturating_sub(used_cols) as f64 * self.step[0] / 2.0, rows.saturating_sub(used_rows) as f64 * self.step[1] / 2.0)
    }
}

// --- the library -----------------------------------------------------------

/// Where saved templates live.
pub fn library_dir() -> PathBuf {
    gtk::glib::user_data_dir().join("spindle").join("templates")
}

/// The templates in a file: one, or two from an older file holding a 16:9
/// and a 4:3 design.
pub fn load(path: &Path) -> anyhow::Result<Vec<CustomTemplate>> {
    let data = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    parse(&data, &path.display().to_string())
}

/// The templates in the contents of file `name`.
pub(super) fn parse(data: &[u8], name: &str) -> anyhow::Result<Vec<CustomTemplate>> {
    let t: CustomTemplate = serde_json::from_slice(data).with_context(|| format!("{name} is not a Spindle template"))?;
    anyhow::ensure!(t.format == FORMAT, "{name} is not a Spindle template");
    anyhow::ensure!(t.version <= 1, "{name} needs a newer version of Spindle");
    Ok(t.split())
}

/// The template of `path` to use on a disc of this shape, for a file
/// holding two.
pub fn load_for(path: &Path, shape: MenuShape) -> anyhow::Result<CustomTemplate> {
    let mut all = load(path)?;
    let i = all.iter().position(|t| t.shape == shape).unwrap_or(0);
    Ok(all.swap_remove(i))
}

pub fn save(t: &CustomTemplate, path: &Path) -> anyhow::Result<()> {
    std::fs::write(path, serde_json::to_vec_pretty(t)?).with_context(|| format!("writing {}", path.display()))
}

/// Saved templates, by name.
pub fn library() -> Vec<(PathBuf, CustomTemplate)> {
    let Ok(rd) = std::fs::read_dir(library_dir()) else { return vec![] };
    let files: Vec<PathBuf> = rd.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == EXTENSION)).collect();
    let mut out = Vec::new();
    for path in files {
        let Ok(mut parts) = load(&path) else { continue };
        if parts.len() == 1 {
            out.push((path, parts.remove(0)));
            continue;
        }
        // A file with a 16:9 and a 4:3 design becomes a file for each.
        let saved: Vec<Option<PathBuf>> = parts.iter().map(|t| save_new(t, &path).ok()).collect();
        if saved.iter().all(Option::is_some) {
            let _ = std::fs::remove_file(&path);
        }
        out.extend(parts.into_iter().zip(saved).map(|(t, new)| (new.unwrap_or_else(|| path.clone()), t)));
    }
    out.sort_by_key(|a| a.1.name.to_lowercase());
    out
}

/// A free file name in the library for `t`.
fn library_path(t: &CustomTemplate) -> PathBuf {
    let dir = library_dir();
    let slug: String = t.name.chars().map(|c| if c.is_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).collect();
    let slug = slug.split('-').filter(|s| !s.is_empty()).collect::<Vec<_>>().join("-");
    let slug = if slug.is_empty() { "template".to_string() } else { slug };
    let mut path = dir.join(format!("{slug}.{EXTENSION}"));
    let mut n = 2;
    while path.exists() {
        path = dir.join(format!("{slug}-{n}.{EXTENSION}"));
        n += 1;
    }
    path
}

/// Write `t` to a new file in the library, beside `old`.
fn save_new(t: &CustomTemplate, old: &Path) -> anyhow::Result<PathBuf> {
    let path = library_path(t);
    anyhow::ensure!(path != old, "no free name");
    save(t, &path)?;
    Ok(path)
}

pub fn find(name: &str) -> Option<CustomTemplate> {
    library().into_iter().map(|(_, t)| t).find(|t| t.name == name)
}

/// Add a template to the library (replacing one of the same name).
pub fn install(t: &CustomTemplate) -> anyhow::Result<PathBuf> {
    let dir = library_dir();
    std::fs::create_dir_all(&dir)?;
    if let Some((old, _)) = library().into_iter().find(|(_, o)| o.name == t.name) {
        let _ = std::fs::remove_file(old);
    }
    let path = library_path(t);
    save(t, &path)?;
    Ok(path)
}

// --- making menus from a template ------------------------------------------

impl Ctx {
    pub(super) fn custom(t: &CustomTemplate) -> Ctx {
        let p = &t.palette;
        let theme = Theme {
            name: "Custom",
            top: p.top,
            bottom: p.bottom,
            panel: p.panel,
            text: p.text,
            dim: p.dim,
            selected: p.selected,
            activated: p.activated,
            shadow: true,
        };
        Ctx { theme, heading: t.fonts.heading.clone(), body: t.fonts.body.clone() }
    }
}

/// Put the template's pictures on disk and in the project; returns the
/// project asset for each template image id.
pub(super) fn install_images(p: &mut Project, t: &CustomTemplate) -> HashMap<Id, Id> {
    let dir = gtk::glib::user_data_dir().join("spindle").join("template-images");
    let _ = std::fs::create_dir_all(&dir);
    let mut out = HashMap::new();
    for (id, img) in &t.images {
        let data = gtk::glib::base64_decode(&img.data);
        let ext = Path::new(&img.name).extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_else(|| "png".into());
        let path = dir.join(format!("{:016x}.{ext}", crate::encode_cache::fnv(&data)));
        if !path.exists() && std::fs::write(&path, &data).is_err() {
            continue;
        }
        let asset = match p.assets.iter().find(|a| a.path == path) {
            Some(a) => a.id,
            None => {
                let info = crate::media::probe::probe(&path).unwrap_or_default();
                let a = Asset { id: new_id(), path: path.clone(), kind: AssetKind::Image, info };
                let aid = a.id;
                p.assets.push(a);
                aid
            }
        };
        out.insert(*id, asset);
    }
    out
}

/// Fill in placeholders; `{TITLE}` gives the value in capitals.
fn fill(text: &str, vars: &[(&str, String)]) -> String {
    let mut s = text.to_string();
    let mut emptied = false;
    for (k, v) in vars {
        for (key, value) in [(format!("{{{k}}}"), v.clone()), (format!("{{{}}}", k.to_uppercase()), v.to_uppercase())] {
            if s.contains(&key) {
                emptied |= value.trim().is_empty();
                s = s.replace(&key, &value);
            }
        }
    }
    if emptied {
        s = tidy(&s);
    }
    s
}

/// Separators left over when a placeholder was empty: "A ·  · B" → "A · B",
/// and none at either end.
fn tidy(s: &str) -> String {
    const SEPARATORS: [char; 4] = ['·', '|', '•', '/'];
    let parts: Vec<&str> = s.split(|c| SEPARATORS.contains(&c)).collect();
    if parts.len() < 2 {
        return s.trim().to_string();
    }
    // Keep the original separator between the parts that remain.
    let sep_char = s.chars().find(|c| SEPARATORS.contains(c)).unwrap_or('·');
    let gap = s.split(sep_char).nth(1).map_or(" ", |rest| if rest.starts_with("  ") { "  " } else { " " });
    parts.iter().map(|p| p.trim()).filter(|p| !p.is_empty()).collect::<Vec<_>>().join(&format!("{gap}{sep_char}{gap}"))
}

/// Put placeholders back for values found in `s` (also in capitals).
fn unfill(s: &str, vars: &[(String, &str)]) -> String {
    let mut s = s.to_string();
    for (value, key) in vars {
        if value.trim().is_empty() {
            continue;
        }
        s = s.replace(value.as_str(), &format!("{{{key}}}"));
        let upper = value.to_uppercase();
        if upper != *value {
            s = s.replace(&upper, &format!("{{{}}}", key.to_uppercase()));
        }
    }
    s
}

/// What the placeholders and links of a menu turn into.
struct Fill<'a> {
    vars: Vec<(&'static str, String)>,
    link: &'a dyn Fn(Link) -> Option<Action>,
    still: &'a dyn Fn(Still) -> Option<(Id, f64)>,
    images: &'a HashMap<Id, Id>,
}

/// Copies of `items` (moved by `offset`), filled in; `ids` maps template
/// item ids to the new ones.
fn make_items(items: &[TemplateItem], offset: (f64, f64), f: &Fill, ids: &mut HashMap<Id, Id>) -> Vec<MenuItem> {
    let mut groups: HashMap<Id, Id> = HashMap::new();
    let mut out = Vec::new();
    for ti in items {
        let mut it = ti.item.clone();
        let new = new_id();
        ids.insert(it.id, new);
        it.id = new;
        it.rect = Rect::new(it.rect.x + offset.0, it.rect.y + offset.1, it.rect.w, it.rect.h);
        if let Some(g) = it.group {
            it.group = Some(*groups.entry(g).or_insert_with(new_id));
        }
        let still = ti.still.and_then(|s| (f.still)(s));
        match &mut it.kind {
            ItemKind::Text(t) => {
                t.text = fill(&t.text, &f.vars);
                if t.text.trim().is_empty() {
                    continue;
                }
            }
            ItemKind::Button(b) => {
                b.label = fill(&b.label, &f.vars);
                if let Some(link) = ti.link {
                    match (f.link)(link) {
                        Some(a) => b.action = a,
                        // A link with nowhere to go (no Setup menu, no next page).
                        None => continue,
                    }
                }
                if let Some((asset, time)) = still {
                    b.thumbnail = Some(asset);
                    b.thumbnail_time = time;
                }
                for i in [&mut b.images.normal, &mut b.images.selected, &mut b.images.activated].into_iter().flatten() {
                    *i = f.images.get(i).copied().unwrap_or(*i);
                }
            }
            ItemKind::Image(img) => match (ti.still, still) {
                (Some(_), Some((asset, time))) => {
                    img.asset = asset;
                    img.time = time;
                }
                (Some(_), None) => continue,
                _ => match f.images.get(&img.asset) {
                    Some(a) => img.asset = *a,
                    None => continue,
                },
            },
            _ => {}
        }
        out.push(it);
    }
    out
}

fn make_menu(tm: &TemplateMenu, name: &str, f: &Fill) -> (Menu, HashMap<Id, Id>) {
    let mut m = Menu::new(name);
    m.background = tm.background.clone();
    m.background.video = None;
    m.background.image = m.background.image.and_then(|i| f.images.get(&i).copied());
    m.fade_in = tm.fade_in;
    m.fade_out = tm.fade_out;
    let mut ids = HashMap::new();
    m.items = make_items(&tm.items, (0.0, 0.0), f, &mut ids);
    m.default_button = tm.default_button.and_then(|d| ids.get(&d).copied()).filter(|d| m.item(*d).is_some());
    (m, ids)
}

/// A list from layout `lay`: its page, and a slot for each of `entries`
/// (placeholders and what the slot's link does). Returns the menu and the
/// first button of each slot. `None` when there are more entries than slots.
fn make_list(lay: &EpisodeLayout, name: &str, page: &Fill, entry: Link, entries: &[(Vec<(&'static str, String)>, Action)]) -> Option<(Menu, Vec<Id>)> {
    if entries.is_empty() || entries.len() > lay.per_page() {
        return None;
    }
    let (mut m, _) = make_menu(&lay.page, name, page);
    let shift = lay.centering(entries.len());
    let mut firsts = Vec::new();
    for (k, (vars, action)) in entries.iter().enumerate() {
        let mut v = page.vars.clone();
        v.extend(vars.iter().cloned());
        let link = |l: Link| if l == entry { Some(*action) } else { (page.link)(l) };
        let f = Fill { vars: v, link: &link, still: page.still, images: page.images };
        let (dx, dy) = lay.offset(k);
        let items = make_items(&lay.slot, (dx + shift.0, dy + shift.1), &f, &mut HashMap::new());
        firsts.push(items.iter().find(|i| i.button().is_some()).map(|i| i.id)?);
        m.items.extend(items);
    }
    Some((m, firsts))
}

/// Move the linked buttons of `m` together where others of their row or
/// column were left out (a pop-up without chapters, say): a row closes up
/// around its middle, a column towards its top.
fn close_gaps(tm: &TemplateMenu, m: &mut Menu, ids: &HashMap<Id, Id>) {
    let linked: Vec<&TemplateItem> = tm.items.iter().filter(|i| i.link.is_some() && i.item.button().is_some()).collect();
    let kept = |i: &TemplateItem| ids.get(&i.item.id).is_some_and(|id| m.item(*id).is_some());
    let mut moves: Vec<(Id, f64, f64)> = Vec::new();
    let mut done: Vec<Id> = Vec::new();
    for horizontal in [true, false] {
        for first in &linked {
            if done.contains(&first.item.id) {
                continue;
            }
            let along = |i: &TemplateItem| if horizontal { (i.item.rect.x, i.item.rect.w) } else { (i.item.rect.y, i.item.rect.h) };
            let across = |i: &TemplateItem| if horizontal { i.item.rect.y } else { i.item.rect.x };
            let mut line: Vec<&TemplateItem> = linked.iter().copied().filter(|i| !done.contains(&i.item.id) && (across(i) - across(first)).abs() < 1.0).collect();
            if line.len() < 2 {
                continue;
            }
            line.sort_by(|a, b| along(a).0.total_cmp(&along(b).0));
            done.extend(line.iter().map(|i| i.item.id));
            let present: Vec<&&TemplateItem> = line.iter().filter(|i| kept(i)).collect();
            if present.len() == line.len() || present.is_empty() {
                continue;
            }
            let gap = along(line[1]).0 - (along(line[0]).0 + along(line[0]).1);
            let (start, end) = (along(line[0]).0, along(line[line.len() - 1]).0 + along(line[line.len() - 1]).1);
            let total: f64 = present.iter().map(|i| along(i).1).sum::<f64>() + gap * (present.len() - 1) as f64;
            let mut at = if horizontal { start + (end - start - total) / 2.0 } else { start };
            for i in present {
                let delta = at - along(i).0;
                moves.push((ids[&i.item.id], if horizontal { delta } else { 0.0 }, if horizontal { 0.0 } else { delta }));
                at += along(i).1 + gap;
            }
        }
    }
    for (id, dx, dy) in moves {
        if let Some(it) = m.items.iter_mut().find(|i| i.id == id) {
            it.rect = Rect::new(it.rect.x + dx, it.rect.y + dy, it.rect.w, it.rect.h);
        }
    }
}

/// The template's own Setup and pop-up menus.
pub(super) struct Extras<'a> {
    setup: Option<&'a EpisodeLayout>,
    popup: Option<&'a PopupLayout>,
    shape: MenuShape,
    vars: Vec<(&'static str, String)>,
    /// The first episode's still.
    first: Option<(Id, f64)>,
    images: &'a HashMap<Id, Id>,
}

/// The placeholders that work anywhere.
fn common_vars(input: &Input) -> Vec<(&'static str, String)> {
    vec![
        ("title", input.name.clone()),
        ("season", input.season.clone()),
        ("disc", input.disc.clone()),
        ("edition", input.edition().unwrap_or_default()),
        ("count", input.count()),
    ]
}

impl<'a> Extras<'a> {
    pub(super) fn new(t: &'a CustomTemplate, input: &Input, images: &'a HashMap<Id, Id>) -> Self {
        Extras { setup: t.setup.as_ref(), popup: t.popup.as_ref(), shape: t.shape, vars: common_vars(input), first: input.episodes.first().map(|e| (e.asset, e.poster)), images }
    }

    fn language_entries(languages: &[LanguagePreset], back: Id) -> Vec<(Vec<(&'static str, String)>, Action)> {
        languages.iter().map(|l| (vec![("name", l.name.clone())], Action::SetLanguage { preset: l.id, menu: Some(back) })).collect()
    }

    /// A list menu, with the button of `default` (or the first) selected.
    fn list(&self, lay: &EpisodeLayout, name: &str, link: &dyn Fn(Link) -> Option<Action>, entry: Link, entries: &[(Vec<(&'static str, String)>, Action)], default: usize) -> Option<Menu> {
        let still = |_: Still| self.first;
        let f = Fill { vars: self.vars.clone(), link, still: &still, images: self.images };
        let (mut m, firsts) = make_list(lay, name, &f, entry, entries)?;
        m.default_button = firsts.get(default).or(firsts.first()).copied();
        m.shape = self.shape;
        Some(m)
    }

    /// The Setup menu, when the template has one that holds every choice.
    pub(super) fn setup(&self, languages: &[LanguagePreset], default: Option<Id>, home: Id) -> Option<Menu> {
        let link = |l: Link| matches!(l, Link::MainMenu | Link::Back).then_some(Action::ShowMenu(home));
        let default = languages.iter().position(|l| Some(l.id) == default).unwrap_or(0);
        self.list(self.setup?, &gettext("Setup"), &link, Link::Language, &Self::language_entries(languages, home), default)
    }

    fn popup_link(input: &PopupInput, chapters: Option<Id>, audio: Option<Id>, back: Option<Id>) -> impl Fn(Link) -> Option<Action> {
        let (episodes, main, setup) = (input.episodes, input.main, input.setup);
        move |l: Link| {
            match l {
                Link::Chapters => chapters,
                Link::Audio => audio,
                Link::Episodes => episodes,
                Link::MainMenu => Some(main),
                Link::Setup => setup,
                Link::Back => back,
                _ => None,
            }
            .map(Action::ShowMenu)
        }
    }

    fn as_popup(mut m: Menu) -> Menu {
        m.popup = true;
        m.background = Menu::new_popup("").background;
        m
    }

    /// The pop-up menu itself.
    pub(super) fn popup_main(&self, input: &PopupInput, chapters: Option<Id>, audio: Option<Id>) -> Option<Menu> {
        let tm = &self.popup?.menu;
        let link = Self::popup_link(input, chapters, audio, None);
        let still = |_: Still| self.first;
        let f = Fill { vars: self.vars.clone(), link: &link, still: &still, images: self.images };
        let (mut m, ids) = make_menu(tm, &tm.name, &f);
        close_gaps(tm, &mut m, &ids);
        if m.default_button.is_none() {
            let first = m.buttons().next().map(|b| b.id);
            m.default_button = first;
        }
        m.shape = self.shape;
        Some(Self::as_popup(m))
    }

    /// The pop-up's page of chapters; chapters past its last slot are left out.
    pub(super) fn popup_chapters(&self, input: &PopupInput, back: Id) -> Option<Menu> {
        let lay = self.popup?.chapters.as_ref()?;
        let link = Self::popup_link(input, None, None, Some(back));
        let entries: Vec<_> = (0..input.max_chapters.min(lay.per_page())).map(|i| (vec![("number", (i + 1).to_string())], Action::PlayChapter(i as u32))).collect();
        self.list(lay, &popup_chapters_name(), &link, Link::Chapter, &entries, 0).map(Self::as_popup)
    }

    /// The pop-up's page of language choices, when it holds them all.
    pub(super) fn popup_audio(&self, input: &PopupInput, back: Id) -> Option<Menu> {
        let lay = self.popup?.audio.as_ref()?;
        let link = Self::popup_link(input, None, None, Some(back));
        self.list(lay, &popup_audio_name(), &link, Link::Language, &Self::language_entries(input.languages, back), 0).map(Self::as_popup)
    }
}

/// Menus from custom template `t`: the main menu, then the episode pages.
pub(super) fn build(t: &CustomTemplate, input: Input, images: &HashMap<Id, Id>) -> Output {
    let common = common_vars(&input);
    let (shape, main_t, lay) = (t.shape, &t.main, &t.episodes);
    let Input { episodes, main: base, setup, .. } = input;
    let per_page = lay.per_page();
    let n_pages = episodes.len().div_ceil(per_page).max(1);
    let page_ids: Vec<Id> = (0..n_pages).map(|_| new_id()).collect();
    let first = episodes.first();

    // Main menu
    let main_id = base.id;
    let link = |l: Link| match l {
        Link::PlayAll => Some(Action::PlayAll),
        Link::Episodes => first.map(|_| Action::ShowMenu(page_ids[0])),
        Link::Setup => setup.map(Action::ShowMenu),
        Link::MainMenu => Some(Action::ShowMenu(main_id)),
        Link::FirstEpisode => first.map(|e| e.action()),
        _ => None,
    };
    let still = |s: Still| match s {
        Still::Nth(n) => episodes.get(n as usize).map(|e| (e.asset, e.poster)),
        Still::First | Still::Page | Still::Episode => first.map(|e| (e.asset, e.poster)),
    };
    let f = Fill { vars: common.clone(), link: &link, still: &still, images };
    let (mut main, ids) = make_menu(main_t, &base.name, &f);
    main.id = main_id;
    main.shape = shape;
    if main.default_button.is_none() {
        let first_button = main.buttons().next().map(|b| b.id);
        main.default_button = first_button;
    }
    // The name, which a logo can take the place of.
    let title_item = main_t
        .items
        .iter()
        .find(|ti| matches!(&ti.item.kind, ItemKind::Text(x) if x.text.contains("{title}")))
        .and_then(|ti| ids.get(&ti.item.id).copied());

    // Episode pages. A layout without Previous / Next buttons (saved from a
    // single page) gets them, like its Main Menu button.
    let mut chrome = lay.page.clone();
    let has = |items: &[TemplateItem], l: Link| items.iter().any(|i| i.link == Some(l));
    if n_pages > 1 && !(has(&chrome.items, Link::Previous) && has(&chrome.items, Link::Next)) {
        let like = chrome.items.iter().find(|i| i.link == Some(Link::MainMenu)).or_else(|| lay.slot.iter().find(|i| i.link == Some(Link::Episode))).cloned();
        if let Some(like) = like {
            let (w, h) = (300.0, like.item.rect.h.min(80.0));
            let y = if like.link == Some(Link::MainMenu) { like.item.rect.y } else { SAFE_AREA.y + SAFE_AREA.h - h };
            for (l, label, x) in [(Link::Previous, gettext("‹ Previous"), SAFE_AREA.x), (Link::Next, gettext("Next ›"), SAFE_AREA.x + SAFE_AREA.w - w)] {
                if has(&chrome.items, l) {
                    continue;
                }
                let mut item = like.item.clone();
                item.id = new_id();
                item.rect = Rect::new(x, y, w, h);
                if let Some(b) = item.button_mut() {
                    b.label = label;
                    b.thumbnail = None;
                }
                chrome.items.push(TemplateItem { item, link: Some(l), still: None });
            }
        }
    }
    let lay = EpisodeLayout { page: chrome, ..lay.clone() };
    let lay = &lay;
    let mut pages = Vec::new();
    for (pi, id) in page_ids.iter().enumerate() {
        let slice: Vec<&Episode> = episodes.iter().skip(pi * per_page).take(per_page).collect();
        let page_first = slice.first().copied();
        let mut vars = common.clone();
        vars.push(("page", (pi + 1).to_string()));
        vars.push(("pages", n_pages.to_string()));
        let label = if n_pages > 1 { gettext("Page {} of {}").replacen("{}", &(pi + 1).to_string(), 1).replacen("{}", &n_pages.to_string(), 1) } else { String::new() };
        vars.push(("page_label", label));
        let nav = |l: Link| match l {
            Link::Previous => pi.checked_sub(1).map(|i| Action::ShowMenu(page_ids[i])),
            Link::Next => page_ids.get(pi + 1).map(|i| Action::ShowMenu(*i)),
            other => link(other),
        };
        let page_still = |s: Still| match s {
            Still::First => first.map(|e| (e.asset, e.poster)),
            Still::Nth(n) => slice.get(n as usize).map(|e| (e.asset, e.poster)),
            _ => page_first.map(|e| (e.asset, e.poster)),
        };
        let heading = if n_pages > 1 { format!("{} {}", gettext("Episodes"), pi + 1) } else { gettext("Episodes") };
        let f = Fill { vars: vars.clone(), link: &nav, still: &page_still, images };
        let (mut page, _) = make_menu(&lay.page, &heading, &f);
        page.id = *id;
        page.shape = shape;
        let chrome_default = page.default_button;
        for (k, e) in slice.iter().enumerate() {
            let mut v = vars.clone();
            v.extend([
                ("number", e.number.to_string()),
                ("number2", format!("{:02}", e.number)),
                ("name", if e.name.is_empty() { gettext("Episode {}").replace("{}", &e.number.to_string()) } else { e.name.clone() }),
                ("title_name", e.name_or_title()),
                ("duration", clock(e.duration)),
                ("minutes", gettext("{} min").replace("{}", &((e.duration / 60.0).round().max(1.0) as u32).to_string())),
            ]);
            let slot_link = |l: Link| match l {
                Link::Episode => Some(e.action()),
                other => nav(other),
            };
            let slot_still = |s: Still| match s {
                Still::Episode => Some((e.asset, e.poster)),
                other => page_still(other),
            };
            let f = Fill { vars: v, link: &slot_link, still: &slot_still, images };
            let mut ids = HashMap::new();
            let items = make_items(&lay.slot, lay.offset(k), &f, &mut ids);
            if k == 0 && chrome_default.is_none() {
                page.default_button = items.iter().find(|i| i.button().is_some()).map(|i| i.id);
            }
            page.items.extend(items);
        }
        if episodes.is_empty() {
            page.items.push(MenuItem::new_text(&gettext("Import your episodes, then apply the template again"), Rect::new(260.0, 480.0, 1400.0, 120.0)));
        }
        pages.push(page);
    }
    let return_to = episodes.iter().enumerate().map(|(i, e)| (e.title, page_ids[(i / per_page).min(n_pages - 1)])).collect();
    let popup_links = match (first, page_ids.first()) {
        (Some(_), Some(p)) => vec![(gettext("Episodes"), Action::ShowMenu(*p))],
        _ => vec![],
    };
    let mut menus = vec![main];
    menus.extend(pages);
    Output { menus, return_to, title_item, popup_links }
}

// --- saving a project's menus as a template --------------------------------

/// Pictures the template needs: `asset` is embedded once.
fn embed(p: &Project, asset: Id, images: &mut BTreeMap<Id, Image>) -> bool {
    if images.contains_key(&asset) {
        return true;
    }
    let Some(a) = p.asset(asset).filter(|a| a.kind == AssetKind::Image) else { return false };
    let Ok(data) = std::fs::read(&a.path) else { return false };
    images.insert(asset, Image { name: a.name(), data: gtk::glib::base64_encode(&data).to_string() });
    true
}

/// Make `m` into a template menu. `link` says what a button's action
/// becomes, `still` which episode a video frame belongs to, and `text`
/// fills placeholders back in.
fn template_menu(
    p: &Project,
    m: &Menu,
    keep: impl Fn(&MenuItem) -> bool,
    link: &dyn Fn(&Action) -> Option<Link>,
    still: &dyn Fn(Id) -> Option<Still>,
    text: &dyn Fn(&str) -> String,
    images: &mut BTreeMap<Id, Image>,
) -> TemplateMenu {
    let mut items = Vec::new();
    for it in m.items.iter().filter(|i| keep(i) && !i.hidden) {
        let mut it = it.clone();
        let mut ti_link = None;
        let mut ti_still = None;
        match &mut it.kind {
            ItemKind::Text(t) => t.text = text(&t.text),
            ItemKind::Button(b) => {
                b.label = text(&b.label);
                ti_link = link(&b.action);
                b.action = Action::None;
                b.nav = Default::default();
                if let Some(v) = b.thumbnail {
                    ti_still = still(v);
                    if ti_still.is_none() {
                        b.thumbnail = None;
                    }
                }
                for img in [&mut b.images.normal, &mut b.images.selected, &mut b.images.activated] {
                    if img.is_some_and(|i| !embed(p, i, images)) {
                        *img = None;
                    }
                }
            }
            ItemKind::Image(img) => {
                let asset = img.asset;
                let video = p.asset(asset).is_some_and(|a| a.kind == AssetKind::Video);
                if video {
                    ti_still = still(asset);
                    if ti_still.is_none() {
                        continue;
                    }
                } else if !embed(p, asset, images) {
                    continue;
                }
            }
            _ => {}
        }
        items.push(TemplateItem { item: it, link: ti_link, still: ti_still });
    }
    let mut background = m.background.clone();
    background.video = None;
    if background.image.is_some_and(|i| !embed(p, i, images)) {
        background.image = None;
    }
    TemplateMenu { name: m.name.clone(), background, items, default_button: m.default_button, fade_in: m.fade_in, fade_out: m.fade_out }
}

/// Make list menu `m` into a layout: its buttons that `is_entry` become one
/// slot (the first of them, labelled `label`), repeated the way they are
/// laid out; the rest is the page.
#[allow(clippy::too_many_arguments)]
fn list_layout(
    p: &Project,
    m: &Menu,
    is_entry: &dyn Fn(&Action) -> bool,
    entry: Link,
    label: &str,
    link: &dyn Fn(&Action) -> Option<Link>,
    still: &dyn Fn(Id) -> Option<Still>,
    text: &dyn Fn(&str) -> String,
    images: &mut BTreeMap<Id, Image>,
) -> Option<EpisodeLayout> {
    let entries: Vec<&MenuItem> = m.buttons().filter(|b| b.button().is_some_and(|b| is_entry(&b.action))).collect();
    let first = *entries.first()?;
    let (r0, n) = (first.rect, entries.len());
    let is = |it: &MenuItem| entries.iter().any(|e| e.id == it.id);
    let page = template_menu(p, m, |it| !is(it), link, still, text, images);
    let mut slot = template_menu(p, m, |it| it.id == first.id, &|_| Some(entry), still, &|_| label.to_string(), images).items;
    let across = entries.get(1).is_none_or(|b| (b.rect.y - r0.y).abs() < 1.0);
    let (columns, rows, step) = if across {
        // Wide enough for the longest label (as far as the row has room
        // for them all), and as many as the row holds.
        let gap = entries.get(1).map_or(24.0, |b| b.rect.x - (r0.x + r0.w)).max(0.0);
        let room = (SAFE_AREA.x + SAFE_AREA.w - r0.x - gap * (n - 1) as f64) / n as f64;
        let w = entries.iter().map(|b| b.rect.w).fold(r0.w, f64::max).min(room.max(r0.w));
        let dx = w + gap;
        slot.iter_mut().for_each(|s| s.item.rect.w = w);
        let fit = ((SAFE_AREA.x + SAFE_AREA.w - r0.x - w) / dx).floor().max(0.0) as usize + 1;
        (fit.max(n), 1, [dx, 0.0])
    } else {
        (1, n, [0.0, entries[1].rect.y - r0.y])
    };
    Some(EpisodeLayout { page, slot, columns: columns as u32, rows: rows as u32, step, order: Order::Rows, center: false })
}

/// Turn the project's main menu and episode pages into a template, with
/// its Setup and pop-up menus when it has them.
pub fn from_project(p: &Project, name: &str, description: &str, author: &str) -> anyhow::Result<CustomTemplate> {
    let pages = episode_pages(p);
    anyhow::ensure!(!pages.is_empty(), "the project has no episode pages (menus named “Episodes”)");
    let info = p.template.clone();
    let main_id = info
        .as_ref()
        .map(|t| t.main)
        .filter(|m| p.menu(*m).is_some())
        .or_else(|| p.menus.iter().find(|m| !m.popup && !pages.contains(&m.id)).map(|m| m.id))
        .context("the project has no main menu")?;
    let main = p.menu(main_id).context("the project has no main menu")?;
    let page0 = p.menu(pages[0]).context("episode page missing")?;
    let is_chapter = |a: &Action| matches!(a, Action::PlayChapter(_));
    let is_language = |a: &Action| matches!(a, Action::SetLanguage { .. });
    let has = |m: &Menu, f: &dyn Fn(&Action) -> bool| m.buttons().any(|b| b.button().is_some_and(|b| f(&b.action)));
    let setup_menu = p.disc_menus().find(|m| has(m, &is_language));
    let setup = setup_menu.map(|m| m.id);
    // The pop-up menu and its pages.
    let popup_chapters = p.popup_menus().find(|m| has(m, &is_chapter));
    let popup_audio = p.popup_menus().find(|m| has(m, &is_language));
    let own_page = |m: &&Menu| !has(m, &is_chapter) && !has(m, &is_language);
    let popup_main = p.disc.popup_menu.and_then(|id| p.menu(id)).filter(|m| m.popup).filter(own_page).or_else(|| p.popup_menus().find(own_page));
    let eps = episodes(p);

    // Text that came from the template's inputs goes back to placeholders.
    let title = info.as_ref().map(|t| t.title.clone()).filter(|t| !t.trim().is_empty()).unwrap_or_else(|| p.disc.name.clone());
    let (season, disc) = info.as_ref().map(|t| (t.season.clone(), t.disc.clone())).unwrap_or_default();
    let edition: Vec<&str> = [season.trim(), disc.trim()].into_iter().filter(|s| !s.is_empty()).collect();
    let edition = edition.join(" · ");
    let n = eps.len() as u32;
    let count = gettextrs::ngettext("{} Episode", "{} Episodes", n).replace("{}", &n.to_string());
    let vars = vec![(edition, "edition"), (title, "title"), (season, "season"), (disc, "disc"), (count, "count")];
    let common = move |s: &str| -> String { unfill(s, &vars) };
    let ep_assets: Vec<Id> = eps.iter().map(|e| e.asset).collect();
    let first_title = eps.first().map(|e| e.title);
    let mut images = BTreeMap::new();

    // Main menu
    let main_link = |a: &Action| match a {
        Action::PlayAll => Some(Link::PlayAll),
        Action::ShowMenu(m) if pages.contains(m) => Some(Link::Episodes),
        Action::ShowMenu(m) if Some(*m) == setup => Some(Link::Setup),
        Action::PlayTitle { title, .. } if Some(*title) == first_title => Some(Link::FirstEpisode),
        _ => None,
    };
    let main_still = |asset: Id| ep_assets.iter().position(|a| *a == asset).map(|k| if k == 0 { Still::First } else { Still::Nth(k as u32) });
    let main_t = template_menu(p, main, |_| true, &main_link, &main_still, &common, &mut images);

    // The episode buttons of the first page, in order.
    let ep_buttons: Vec<&MenuItem> = page0.buttons().filter(|b| matches!(b.button().map(|b| b.action), Some(Action::PlayTitle { .. }))).collect();
    anyhow::ensure!(!ep_buttons.is_empty(), "the first episode page has no episode buttons");
    let r0 = ep_buttons[0].rect;
    let same_row = ep_buttons.iter().filter(|b| (b.rect.y - r0.y).abs() < 1.0).count();
    let same_col = ep_buttons.iter().filter(|b| (b.rect.x - r0.x).abs() < 1.0).count();
    // The second episode below the first: the list runs down, then across.
    let down_first = ep_buttons.get(1).is_some_and(|b| (b.rect.x - r0.x).abs() < 1.0 && b.rect.y > r0.y);
    let (order, columns, rows) = if !down_first {
        (Order::Rows, same_row.max(1), ep_buttons.len().div_ceil(same_row.max(1)))
    } else {
        (Order::Columns, ep_buttons.len().div_ceil(same_col), same_col)
    };
    // The step to the next column and row.
    let dx = match order {
        Order::Rows => ep_buttons.get(1).filter(|_| columns > 1).map_or(0.0, |b| b.rect.x - r0.x),
        Order::Columns => ep_buttons.get(rows).map_or(0.0, |b| b.rect.x - r0.x),
    };
    let dy = match order {
        Order::Rows => ep_buttons.get(columns).map_or(0.0, |b| b.rect.y - r0.y),
        Order::Columns => ep_buttons.get(1).map_or(0.0, |b| b.rect.y - r0.y),
    };
    // Each episode's cell: its button widened to the step (or the screen).
    let cell = |b: &Rect| {
        let (x, w) = if dx > 0.0 { (b.x, dx) } else { (0.0, DESIGN_WIDTH) };
        let h = if dy > 0.0 { dy } else { b.h };
        Rect::new(x, b.y, w, h)
    };
    let cells: Vec<Rect> = ep_buttons.iter().map(|b| cell(&b.rect)).collect();
    // An item is part of an episode when it fits in that episode's cell.
    let in_cell = |it: &MenuItem| -> Option<usize> {
        let r = &it.rect;
        cells.iter().position(|c| r.x >= c.x - 4.0 && r.y >= c.y - 4.0 && r.x + r.w <= c.x + c.w + 4.0 && r.y + r.h <= c.y + c.h + 4.0)
    };
    let is_episode_button = |it: &MenuItem| ep_buttons.iter().any(|b| b.id == it.id);
    // The page's episodes, in order, and which of them an item pictures.
    let page_eps: Vec<&Episode> = ep_buttons
        .iter()
        .filter_map(|b| match b.button().map(|b| b.action) {
            Some(Action::PlayTitle { title, .. }) => eps.iter().find(|e| e.title == title),
            _ => None,
        })
        .collect();
    let pictures = |it: &MenuItem| -> Option<usize> {
        let asset = match &it.kind {
            ItemKind::Image(i) => Some(i.asset),
            ItemKind::Button(b) => b.thumbnail,
            _ => None,
        }?;
        page_eps.iter().position(|e| e.asset == asset)
    };
    // Where each item goes: the slot (repeated per episode), the page, or
    // nowhere (a copy belonging to another episode's slot).
    #[derive(PartialEq)]
    enum Place {
        Slot,
        Page,
        Repeat,
    }
    // A copy of `it` (same kind and size) where episode `to`'s cell puts
    // it, for an item in episode `from`'s cell.
    let twin = |it: &MenuItem, from: usize, to: usize| -> bool {
        let (a, b) = (&ep_buttons[from].rect, &ep_buttons[to].rect);
        let (ox, oy) = (b.x - a.x, b.y - a.y);
        let kind = std::mem::discriminant(&it.kind);
        page0.items.iter().any(|j| {
            j.id != it.id
                && std::mem::discriminant(&j.kind) == kind
                && (j.rect.w - it.rect.w).abs() < 2.0
                && (j.rect.h - it.rect.h).abs() < 2.0
                && (j.rect.x - (it.rect.x + ox)).abs() < 2.0
                && (j.rect.y - (it.rect.y + oy)).abs() < 2.0
        })
    };
    let place = |it: &MenuItem| -> Place {
        if is_episode_button(it) {
            return if it.id == ep_buttons[0].id { Place::Slot } else { Place::Repeat };
        }
        let cell = in_cell(it);
        // Part of each episode only when the neighbouring episode has the
        // same item in the same place (not e.g. lines across the page).
        let repeated = |c: usize| {
            let other = if c == 0 { 1 } else { 0 };
            ep_buttons.len() < 2 || twin(it, c, other)
        };
        match (pictures(it), cell) {
            (Some(k), Some(c)) if k == c => {
                if k == 0 {
                    Place::Slot
                } else {
                    Place::Repeat
                }
            }
            (Some(_), _) => Place::Page,
            (None, Some(0)) if repeated(0) => Place::Slot,
            (None, Some(c)) if c > 0 && repeated(c) => Place::Repeat,
            _ => Place::Page,
        }
    };
    let e0 = &eps.iter().find(|e| Some(e.title) == ep_buttons[0].button().and_then(|b| match b.action {
        Action::PlayTitle { title, .. } => Some(title),
        _ => None,
    }));
    let e0 = e0.or(eps.first()).context("no episodes")?;
    let slot_text = |s: &str| -> String {
        let mut s = s.to_string();
        let name = if e0.name.is_empty() { gettext("Episode {}").replace("{}", &e0.number.to_string()) } else { e0.name.clone() };
        let minutes = gettext("{} min").replace("{}", &((e0.duration / 60.0).round().max(1.0) as u32).to_string());
        s = unfill(&s, &[(e0.name_or_title(), "title_name"), (name, "name"), (clock(e0.duration), "duration"), (minutes, "minutes")]);
        let n2 = format!("{:02}", e0.number);
        if let Some(rest) = s.strip_prefix(&n2).filter(|r| !r.starts_with(|c: char| c.is_ascii_digit())) {
            s = format!("{{number2}}{rest}");
        } else if let Some(rest) = s.strip_prefix(&e0.number.to_string()).filter(|r| !r.starts_with(|c: char| c.is_ascii_digit())) {
            s = format!("{{number}}{rest}");
        }
        common(&s)
    };
    let prev_or_next = |a: &Action, pi: usize| match a {
        Action::ShowMenu(m) if Some(m) == pages.get(pi + 1) => Some(Link::Next),
        Action::ShowMenu(m) if pi > 0 && Some(m) == pages.get(pi - 1) => Some(Link::Previous),
        Action::ShowMenu(m) if *m == main_id => Some(Link::MainMenu),
        other => main_link(other),
    };
    let page_link = |a: &Action| prev_or_next(a, 0);
    let slot_link = |a: &Action| match a {
        Action::PlayTitle { .. } => Some(Link::Episode),
        other => page_link(other),
    };
    let page_still = |asset: Id| page_eps.iter().position(|e| e.asset == asset).map(|k| if k == 0 { Still::Page } else { Still::Nth(k as u32) }).or(ep_assets.contains(&asset).then_some(Still::First));
    let slot_still = |asset: Id| ep_assets.contains(&asset).then_some(Still::Episode);
    let page_label = |s: &str| -> String {
        let label = gettext("Page {} of {}").replacen("{}", "1", 1).replacen("{}", &pages.len().to_string(), 1);
        common(&unfill(s, &[(label, "page_label")]))
    };
    let mut page_t = template_menu(p, page0, |it| place(it) == Place::Page, &page_link, &page_still, &page_label, &mut images);
    page_t.name = gettext("Episodes");
    let slot_t = template_menu(p, page0, |it| place(it) == Place::Slot, &slot_link, &slot_still, &slot_text, &mut images);
    // The first page has no Previous button: take it from the second.
    if let Some(page1) = pages.get(1).and_then(|id| p.menu(*id)) {
        let back = page1.buttons().find(|b| matches!(b.button().map(|b| b.action), Some(Action::ShowMenu(m)) if m == pages[0]));
        if let Some(b) = back {
            let mut item = b.clone();
            if let Some(bb) = item.button_mut() {
                bb.action = Action::None;
                bb.nav = Default::default();
            }
            page_t.items.push(TemplateItem { item, link: Some(Link::Previous), still: None });
        }
    }
    page_t.default_button = None;

    // The Setup and pop-up menus.
    let extra_link = |a: &Action| match a {
        Action::ShowMenu(m) if Some(*m) == popup_chapters.map(|m| m.id) => Some(Link::Chapters),
        Action::ShowMenu(m) if Some(*m) == popup_audio.map(|m| m.id) => Some(Link::Audio),
        Action::ShowMenu(m) if Some(*m) == popup_main.map(|m| m.id) => Some(Link::Back),
        Action::ShowMenu(m) if *m == main_id => Some(Link::MainMenu),
        other => main_link(other),
    };
    let setup_t = setup_menu.and_then(|m| list_layout(p, m, &is_language, Link::Language, "{name}", &extra_link, &main_still, &common, &mut images));
    let popup_t = popup_main.map(|m| PopupLayout {
        menu: template_menu(p, m, |_| true, &extra_link, &main_still, &common, &mut images),
        chapters: popup_chapters.and_then(|m| list_layout(p, m, &is_chapter, Link::Chapter, "{number}", &extra_link, &main_still, &common, &mut images)),
        audio: popup_audio.and_then(|m| list_layout(p, m, &is_language, Link::Language, "{name}", &extra_link, &main_still, &common, &mut images)),
    });

    // Colours and fonts for Setup and the pop-ups Spindle makes itself.
    let style = info.as_ref().and_then(|t| Style::from_id(&t.style));
    let theme = THEMES[info.as_ref().map_or_else(|| detect_theme(p), |t| t.theme).min(THEMES.len() - 1)];
    let (heading, body) = style.map_or(("Cantarell", "Cantarell"), |s| s.fonts());
    let episodes = EpisodeLayout { page: page_t, slot: slot_t.items, columns: columns as u32, rows: rows as u32, step: [dx, dy], order, center: false };
    Ok(CustomTemplate {
        format: FORMAT.into(),
        version: 1,
        name: name.into(),
        description: description.into(),
        author: author.into(),
        palette: Palette {
            top: theme.top,
            bottom: theme.bottom,
            panel: theme.panel,
            text: theme.text,
            dim: theme.dim,
            selected: theme.selected,
            activated: theme.activated,
        },
        fonts: Fonts { heading: heading.into(), body: body.into() },
        shape: main.shape,
        main: main_t,
        episodes,
        setup: setup_t,
        popup: popup_t,
        standard: None,
        images,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_placeholders_leave_no_separators() {
        let vars = [("edition", "Season 3 · Disc 2".to_string()), ("page_label", String::new())];
        assert_eq!(fill("{EDITION}  ·  {PAGE_LABEL}", &vars), "SEASON 3 · DISC 2");
        let vars = [("edition", String::new()), ("page_label", "Page 1 of 2".to_string())];
        assert_eq!(fill("{edition}  ·  {page_label}", &vars), "Page 1 of 2");
        assert_eq!(fill("{title}", &[("title", "Spindle".to_string())]), "Spindle");
    }
}
