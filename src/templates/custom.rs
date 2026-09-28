// SPDX-License-Identifier: GPL-3.0-or-later

//! Custom templates: a main menu and an episode page layout saved from a
//! project (or written by hand) as one `.spindle-template` JSON file, with
//! its pictures inside, to use again on other shows or share.
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
    pub main: TemplateMenu,
    pub episodes: EpisodeLayout,
    /// The same menus designed for 4:3 screens (in the middle of the
    /// canvas), used on 4:3 discs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub standard: Option<Standard>,
    /// Pictures used by items and backgrounds, by the id they use.
    #[serde(default)]
    pub images: BTreeMap<Id, Image>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Standard {
    pub main: TemplateMenu,
    pub episodes: EpisodeLayout,
}

impl CustomTemplate {
    /// Whether the template has menus made for 4:3 screens.
    pub fn has_standard(&self) -> bool {
        self.standard.is_some()
    }

    /// Put the design of `other` for the shape it was saved from (the 4:3
    /// one when it has only that) into this template.
    pub fn combine(mut self, other: &CustomTemplate) -> CustomTemplate {
        match &other.standard {
            // Saved from 4:3 menus: its 4:3 design joins this one.
            Some(s) if s.main == other.main => self.standard = Some(s.clone()),
            _ => {
                self.main = other.main.clone();
                self.episodes = other.episodes.clone();
            }
        }
        self.name = other.name.clone();
        if !other.description.is_empty() {
            self.description = other.description.clone();
        }
        if !other.author.is_empty() {
            self.author = other.author.clone();
        }
        self.palette = other.palette;
        self.fonts = other.fonts.clone();
        self.images.extend(other.images.clone());
        self
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
}

// --- the library -----------------------------------------------------------

/// Where saved templates live.
pub fn library_dir() -> PathBuf {
    gtk::glib::user_data_dir().join("spindle").join("templates")
}

pub fn load(path: &Path) -> anyhow::Result<CustomTemplate> {
    let data = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let t: CustomTemplate = serde_json::from_slice(&data).with_context(|| format!("{} isn't a Spindle template", path.display()))?;
    anyhow::ensure!(t.format == FORMAT, "{} isn't a Spindle template", path.display());
    anyhow::ensure!(t.version <= 1, "{} needs a newer version of Spindle", path.display());
    Ok(t)
}

pub fn save(t: &CustomTemplate, path: &Path) -> anyhow::Result<()> {
    std::fs::write(path, serde_json::to_vec_pretty(t)?).with_context(|| format!("writing {}", path.display()))
}

/// Saved templates, by name.
pub fn library() -> Vec<(PathBuf, CustomTemplate)> {
    let Ok(rd) = std::fs::read_dir(library_dir()) else { return vec![] };
    let mut out: Vec<(PathBuf, CustomTemplate)> = rd
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == EXTENSION))
        .filter_map(|p| load(&p).ok().map(|t| (p, t)))
        .collect();
    out.sort_by_key(|a| a.1.name.to_lowercase());
    out
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
    let slug: String = t.name.chars().map(|c| if c.is_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).collect();
    let slug = slug.split('-').filter(|s| !s.is_empty()).collect::<Vec<_>>().join("-");
    let mut path = dir.join(format!("{}.{EXTENSION}", if slug.is_empty() { "template" } else { &slug }));
    let mut n = 2;
    while path.exists() {
        path = dir.join(format!("{slug}-{n}.{EXTENSION}"));
        n += 1;
    }
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

/// Menus from custom template `t`: the main menu, then the episode pages.
pub(super) fn build(t: &CustomTemplate, input: Input, images: &HashMap<Id, Id>) -> Output {
    let (edition, count) = (input.edition().unwrap_or_default(), input.count());
    // The 4:3 design on 4:3 discs, when there is one.
    let standard = t.standard.as_ref().filter(|_| input.p.disc.video.is_4x3());
    let shape = if standard.is_some() { MenuShape::Standard } else { MenuShape::Wide };
    let (main_t, lay) = match standard {
        Some(s) => (&s.main, &s.episodes),
        None => (&t.main, &t.episodes),
    };
    let Input { name, season, disc, episodes, main: base, setup, .. } = input;
    let per_page = lay.per_page();
    let n_pages = episodes.len().div_ceil(per_page).max(1);
    let page_ids: Vec<Id> = (0..n_pages).map(|_| new_id()).collect();
    let common = vec![("title", name.clone()), ("season", season.clone()), ("disc", disc.clone()), ("edition", edition.clone()), ("count", count.clone())];
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

/// Turn the project's main menu and episode pages into a template.
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
    let setup = p.menus.iter().find(|m| m.buttons().any(|b| matches!(b.button().map(|b| b.action), Some(Action::SetLanguage { .. })))).map(|m| m.id);
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

    // Colours and fonts for Setup and the pop-ups.
    let style = info.as_ref().and_then(|t| Style::from_id(&t.style));
    let theme = THEMES[info.as_ref().map_or_else(|| detect_theme(p), |t| t.theme).min(THEMES.len() - 1)];
    let (heading, body) = style.map_or(("Cantarell", "Cantarell"), |s| s.fonts());
    let episodes = EpisodeLayout { page: page_t, slot: slot_t.items, columns: columns as u32, rows: rows as u32, step: [dx, dy], order };
    // Menus made for 4:3 are the 4:3 design, and stand in for the 16:9
    // one until that is saved too (see `combine`).
    let standard = (main.shape == MenuShape::Standard).then(|| Standard { main: main_t.clone(), episodes: episodes.clone() });
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
        main: main_t,
        episodes,
        standard,
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
