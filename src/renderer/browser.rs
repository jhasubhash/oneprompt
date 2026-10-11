//! Drawing the file browser (crate::browser) in its strip beside a pane, in
//! the pane's own ink so the theme lays it down as it does the terminal:
//! the folder's place as a row of crumbs at the top, a line per entry in
//! the theme's font and inks, marked by kind as `ls -F` marks them (folders
//! with a slash, programs with a star, links with an at sign), the chosen
//! one lit, and about it and the keys at the bottom. Only the rows in view
//! are laid out.

use std::path::{Path, PathBuf};

use alacritty_terminal::event::EventListener;
use glyphon::{Attrs, Buffer, Color, Family, Metrics, Shaping, TextBounds, Weight, Wrap};

use super::{PaneView, Renderer, TITLE_STRIP};
use crate::browser::{self, Browser, Kind};
use crate::theme::{self as api, Shape, Variant};

/// What in a browser is under the pointer.
#[derive(Clone, Debug, PartialEq)]
pub enum BrowserHit {
    /// An entry, by its place in the list shown.
    Row(usize),
    /// One of the folders the shown one is in.
    Crumb(PathBuf),
}

/// A browser as drawn: how far its list is scrolled and where things are.
#[derive(Default)]
pub(super) struct BrowserLook {
    scroll: f32,
    /// The entry and folder last brought into view, to scroll again only
    /// when the choice moves.
    shown_selected: Option<(usize, PathBuf, u32)>,
    targets: Vec<([f32; 4], BrowserHit)>,
    bounds: [f32; 4],
    visible: f32,
    content: f32,
    pub(super) texts: Vec<Buffer>,
    /// The text laid out this frame: buffer, corner, and the box it is cut to.
    pub(super) placed: Vec<(usize, f32, f32, TextBounds)>,
}

impl Renderer {
    /// What in a browser is at (x, y), physical pixels, and whose it is.
    pub fn browser_hit(&self, x: f32, y: f32) -> Option<(u64, BrowserHit)> {
        let inside = |[bx, by, bw, bh]: [f32; 4]| x >= bx && x < bx + bw && y >= by && y < by + bh;
        self.browsers
            .iter()
            .find_map(|(id, look)| look.targets.iter().find(|(r, _)| inside(*r)).map(|(_, hit)| (*id, hit.clone())))
    }

    /// The pane whose browser is at (x, y).
    pub fn browser_at(&self, x: f32, y: f32) -> Option<u64> {
        self.browsers.iter().find_map(|(id, look)| {
            let [bx, by, bw, bh] = look.bounds;
            (x >= bx && x < bx + bw && y >= by && y < by + bh).then_some(*id)
        })
    }

    /// Scrolls pane `id`'s browser by `pixels` (positive: further down).
    pub fn scroll_browser(&mut self, id: u64, pixels: f32) {
        if let Some(look) = self.browsers.get_mut(&id) {
            look.scroll = (look.scroll + pixels).clamp(0.0, (look.content - look.visible).max(0.0));
        }
    }

    /// Forgets the browsers of panes that have none open now.
    pub(super) fn keep_browsers<L: EventListener>(&mut self, panes: &[PaneView<L>]) {
        self.browsers_moving = false;
        let open: Vec<u64> = panes.iter().filter(|p| p.browser.is_some()).map(|p| p.id).collect();
        self.browsers.retain(|id, _| open.contains(id));
    }

    /// Lays out pane `id`'s browser in `rect`: its text for the pane's ink
    /// layer (kept for `draw`), and the marks beneath the ink (returned, for
    /// the surface: the chosen entry's light, the rules, the scroll bar).
    /// Seconds the theme's panel entrance takes on the clock, which the
    /// browser comes in and goes out with; 0 without animation.
    pub fn browser_time(&self, variant: &Variant) -> f32 {
        if !self.animation.0 || self.thumbnail {
            return 0.0;
        }
        variant.theme.panel_entrance().1.clamp(0.05, super::SETTINGS_ENTRANCE_MAX) / self.animation.1
    }

    /// How far in a browser is, 0..1 (before easing): coming in since it
    /// opened, or going out since it began to close, from where it had got to.
    fn browser_progress(&self, variant: &Variant, b: &Browser) -> f32 {
        let d = self.browser_time(variant);
        if d <= 0.0 {
            return if b.closing.is_some() { 0.0 } else { 1.0 };
        }
        match b.closing {
            Some(c) => {
                let had = (c.saturating_duration_since(b.opened).as_secs_f32() / d).clamp(0.0, 1.0);
                (had - c.elapsed().as_secs_f32() / d).clamp(0.0, 1.0)
            }
            None => (b.opened.elapsed().as_secs_f32() / d).clamp(0.0, 1.0),
        }
    }

    /// Lays out pane `id`'s browser in `rect`, as far in as it has come:
    /// its text for the pane's ink layer (kept for `draw`); the marks
    /// beneath the ink (the chosen entry's light, the rules, the scroll
    /// bar); and the theme's moving edge over it while it comes or goes.
    pub(super) fn lay_out_browser(
        &mut self,
        variant: &Variant,
        id: u64,
        b: &Browser,
        rect: [f32; 4],
        focused: bool,
    ) -> (Vec<Shape>, Vec<Shape>) {
        let raw = self.browser_progress(variant, b);
        if raw < 1.0 {
            self.browsers_moving = true;
        }
        let mut look = self.browsers.remove(&id).unwrap_or_default();
        if b.closing.is_some() && raw <= 0.0 {
            // Gone: the app takes it away.
            look.placed.clear();
            look.targets.clear();
            look.bounds = [0.0; 4];
            self.browsers.insert(id, look);
            return (Vec::new(), Vec::new());
        }
        let s = self.window.scale_factor() as f32;
        let w = self.config.width as f32;
        // Below the title strip, where the tabs are.
        let strip = TITLE_STRIP * s;
        let rect = if rect[1] < strip { [rect[0], strip, rect[2], (rect[3] - (strip - rect[1])).max(1.0)] } else { rect };
        let [bx, by, bw, bh] = rect;
        let pad = 14.0 * s;
        let (x0, width) = (bx + pad, (bw - 2.0 * pad).max(1.0));
        let top0 = by + 6.0 * s;
        let (ink, accent, quiet, surface) = (variant.ink, variant.cursor, variant.palette[8], variant.surface_rgb());
        let (red, green, blue, magenta, cyan) =
            (variant.palette[1], variant.palette[2], variant.palette[4], variant.palette[5], variant.palette[6]);
        let has_keys = b.keys && focused;
        let family = variant.font_family.clone();
        // Names in the terminal's own weight, so they read as its text does;
        // what is said about them lighter.
        // The terminal's own font: its face, weight and size (zoomed with
        // it), smaller only for what is said about the entries.
        let weight = self.weight_for(&family, variant.font_weight);
        let (regular, names, bold) = (weight, weight, weight);
        let base = variant.font_size * self.zoom;
        let attrs = |rgb: [u8; 3], weight: u16| {
            let [r, g, bl] = rgb;
            Attrs::new().family(Family::Name(&family)).weight(Weight(weight)).color(Color::rgb(r, g, bl))
        };

        // `put` shapes one piece of text into the next free buffer: its
        // index, width and height.
        let fs = &mut self.font_system;
        let texts = &mut look.texts;
        let mut used = 0;
        let mut put = |spans: &[(&str, Attrs)], size: f32, wrap_width: Option<f32>| {
            if used == texts.len() {
                texts.push(Buffer::new(fs, Metrics::new(12.0, 16.0)));
            }
            let line = (size * 1.35 * s).round();
            let mut buffer = texts[used].borrow_with(fs);
            buffer.set_metrics(Metrics::new(size * s, line));
            buffer.set_wrap(if wrap_width.is_some() { Wrap::Word } else { Wrap::None });
            buffer.set_size(Some(wrap_width.unwrap_or(w)), None);
            buffer.set_rich_text(spans.iter().map(|(t, a)| (*t, a.clone())), &spans[0].1, Shaping::Advanced, None);
            buffer.shape_until_scroll(false);
            let (mut text_w, mut lines) = (0.0f32, 0);
            for run in buffer.layout_runs() {
                text_w = text_w.max(run.line_w);
                lines += 1;
            }
            used += 1;
            (used - 1, text_w, lines.max(1) as f32 * line)
        };
        // Text placed: buffer, corner, and the box it is cut to.
        let mut placed: Vec<(usize, f32, f32, [f32; 4])> = Vec::new();
        let whole = [bx, by, bw, bh];
        let line_w = (1.0 * s).max(1.0);
        let mut shapes = vec![
            // Calmed a touch, so the names read over the paper's marks.
            Shape::new(bx, by, bw, bh, surface, 0.15),
            Shape::new(bx, top0 - 4.0 * s, line_w, by + bh - top0 + 4.0 * s, quiet, if has_keys { 0.6 } else { 0.35 }),
        ];
        let mut targets: Vec<([f32; 4], BrowserHit)> = Vec::new();

        // The crumbs: the folders the shown one is in, then it, as many as
        // fit from the end. Each takes the browser there.
        let mut y = top0 + 6.0 * s;
        let crumbs = crumbs(&b.dir);
        let sep = put(&[(" / ", attrs(quiet, regular))], base * 0.9, None);
        let mut pieces = Vec::new();
        for (i, (label, _)) in crumbs.iter().enumerate() {
            let last = i + 1 == crumbs.len();
            pieces.push(put(&[(label, attrs(if last { ink } else { quiet }, if last { bold } else { regular }))], base * 0.9, None));
        }
        let mut first = 0;
        let fits = |from: usize, pieces: &[(usize, f32, f32)]| {
            pieces[from..].iter().map(|p| p.1).sum::<f32>() + sep.1 * (pieces.len() - from) as f32 <= width
        };
        while first + 1 < pieces.len() && !fits(first, &pieces) {
            first += 1;
        }
        let mut x = x0;
        let crumb_h = pieces.last().map_or(16.0 * s, |p| p.2);
        if first > 0 {
            let (more, more_w, _) = put(&[("\u{2026}", attrs(quiet, regular))], base * 0.9, None);
            placed.push((more, x, y, whole));
            x += more_w;
            placed.push((sep.0, x, y, whole));
            x += sep.1;
        }
        for (i, &(buffer, piece_w, piece_h)) in pieces.iter().enumerate().skip(first) {
            placed.push((buffer, x, y, [x0, by, width, bh]));
            targets.push(([x - 2.0 * s, y - 2.0 * s, piece_w + 4.0 * s, piece_h + 4.0 * s], BrowserHit::Crumb(crumbs[i].1.clone())));
            x += piece_w;
            // The root is its own separator.
            if i + 1 < pieces.len() && crumbs[i].0 != "/" {
                // The separators share one buffer: placed again at each.
                placed.push((sep.0, x, y, whole));
                x += sep.1;
            }
        }
        y += crumb_h + 4.0 * s;

        // A name being typed, or the filter, or why the folder can't be read.
        let mut line_under = |text: &str, rgb: [u8; 3], caret: bool, y: &mut f32, shapes: &mut Vec<Shape>, placed: &mut Vec<_>| {
            let (t, t_w, t_h) = put(&[(text, attrs(rgb, regular))], base * 0.85, if caret { None } else { Some(width) });
            placed.push((t, x0, *y, [x0, by, width, bh]));
            if caret {
                let cx = (x0 + t_w + 1.0 * s).min(x0 + width - 2.0 * s);
                shapes.push(Shape::new(cx, *y + 2.0 * s, (1.5 * s).max(1.0), t_h - 4.0 * s, accent, 1.0));
            }
            *y += t_h + 2.0 * s;
        };
        if let Some(naming) = &b.naming {
            line_under(&format!("{}: {}", naming.prompt(), naming.text()), accent, true, &mut y, &mut shapes, &mut placed);
        } else if !b.filter.is_empty() {
            let text = format!("\u{2315} {}", b.filter);
            line_under(&text, ink, has_keys, &mut y, &mut shapes, &mut placed);
        }
        if let Some(error) = &b.error {
            line_under(&format!("can't read this folder: {error}"), red, false, &mut y, &mut shapes, &mut placed);
        }
        y += 4.0 * s;
        shapes.push(Shape::new(x0, y, width, line_w, quiet, 0.3));
        let rows_top = y + 4.0 * s;

        // The footer, from the bottom up: the keys, then about the chosen entry.
        let bottom_edge = by + bh - 10.0 * s;
        // Each key stays with what it does when the line wraps.
        let keys_text = if has_keys {
            ["\u{21a9} open", "\u{2318}\u{21a9} cd", "\u{21e5} insert", "\u{2318}C copy path", "space look", if b.can_go_back() { "esc back" } else { "esc close" }]
                .map(|k| k.replace(' ', "\u{a0}"))
                .join("  ")
        } else {
            "\u{2318}B or a click to browse".to_string()
        };
        let (keys, _, keys_h) = put(&[(&keys_text, attrs(quiet, regular))], base * 0.72, Some(width));
        let keys_y = bottom_edge - keys_h;
        placed.push((keys, x0, keys_y, whole));
        let about = match b.current() {
            Some(e) => {
                let mut parts = Vec::new();
                parts.push(match e.kind {
                    Kind::Folder => "folder".to_string(),
                    Kind::Link(true) => "link to a folder".to_string(),
                    Kind::Link(false) => format!("link, {}", browser::size_text(e.size)),
                    Kind::File => browser::size_text(e.size),
                });
                if let Some(time) = e.modified {
                    parts.push(browser::when_text(time));
                }
                parts.join(" \u{b7} ")
            }
            None if b.filter.is_empty() => "empty".to_string(),
            None => "nothing matches".to_string(),
        };
        let (total, hidden) = b.counts();
        let items = |n: usize| if n == 1 { "1 item".to_string() } else { format!("{n} items") };
        let count = if b.len() < total { format!("{} of {}", b.len(), items(total)) } else { items(total) };
        let count = if hidden > 0 && !b.show_hidden { format!("{count} \u{b7} {hidden} hidden") } else { count };
        let (about_t, _, about_h) = put(&[(&about, attrs(ink, regular))], base * 0.8, Some(width));
        let (count_t, _, count_h) = put(&[(&count, attrs(quiet, regular))], base * 0.8, Some(width));
        let count_y = keys_y - 6.0 * s - count_h;
        let about_y = count_y - about_h;
        placed.push((about_t, x0, about_y, whole));
        placed.push((count_t, x0, count_y, whole));
        let rows_bottom = about_y - 8.0 * s;
        shapes.push(Shape::new(x0, rows_bottom + 2.0 * s, width, line_w, quiet, 0.3));

        // The rows, scrolled so the chosen one is in view when it moves.
        let row_h = (variant.line_height * self.zoom * s).round() + 2.0 * s;
        let visible = (rows_bottom - rows_top).max(row_h);
        let content = b.len() as f32 * row_h;
        let key = (b.selected, b.dir.clone(), visible as u32);
        if look.shown_selected.as_ref() != Some(&key) {
            let (a, z) = (b.selected as f32 * row_h, (b.selected + 1) as f32 * row_h);
            if a < look.scroll {
                look.scroll = a;
            } else if z > look.scroll + visible {
                look.scroll = z - visible;
            }
            look.shown_selected = Some(key);
        }
        look.scroll = look.scroll.clamp(0.0, (content - visible).max(0.0));
        let first_row = (look.scroll / row_h).floor() as usize;
        let last_row = (((look.scroll + visible) / row_h).ceil() as usize).min(b.len());
        let rows_box = [bx, rows_top, bw, visible];
        let show_sizes = width >= 200.0 * s;
        for (i, e) in b.shown().enumerate().take(last_row).skip(first_row) {
            let ry = rows_top + i as f32 * row_h - look.scroll;
            let row = [bx + 6.0 * s, ry, bw - 12.0 * s, row_h];
            if i == b.selected {
                let (rgb, alpha) = if has_keys { (accent, 0.16) } else { (quiet, 0.14) };
                shapes.push(Shape::rounded(row, 5.0 * s, 0.0, rgb, alpha));
            }
            let (rgb, mark) = match e.kind {
                Kind::Folder => (blue, "/"),
                Kind::Link(true) => (cyan, "/@"),
                Kind::Link(false) => (cyan, "@"),
                Kind::File if e.executable => (green, "*"),
                Kind::File => match browser::opener(Path::new(&e.name)) {
                    browser::Opener::Picture => (magenta, ""),
                    _ => (ink, ""),
                },
            };
            let rgb = if e.hidden() { quiet } else { rgb };
            let weight = if e.is_folder() { bold } else { names };
            // The size on the right, the name cut short before it.
            let mut name_w = width;
            if show_sizes && !e.is_folder() {
                let (z, z_w, z_h) = put(&[(&browser::size_text(e.size), attrs(quiet, regular))], base * 0.72, None);
                placed.push((z, x0 + width - z_w, ry + (row_h - z_h) / 2.0, rows_box));
                name_w = width - z_w - 8.0 * s;
            }
            let (t, _, t_h) = put(&[(&e.name, attrs(rgb, weight)), (mark, attrs(quiet, regular))], base, None);
            let ty = ry + (row_h - t_h) / 2.0;
            placed.push((t, x0, ty, [x0, rows_top, name_w, visible]));
            let (a, z) = (ry.max(rows_top), (ry + row_h).min(rows_top + visible));
            if z > a {
                targets.push(([bx, a, bw, z - a], BrowserHit::Row(i)));
            }
        }
        if content > visible {
            let bar_h = (visible * visible / content).max(20.0 * s);
            let bar_y = rows_top + (visible - bar_h) * look.scroll / (content - visible);
            shapes.push(Shape::rounded([bx + bw - 6.0 * s, bar_y, 3.0 * s, bar_h], 1.5 * s, 0.0, quiet, 0.4));
        }
        (look.visible, look.content) = (visible, content);

        // The theme's entrance, as the settings panel has it: unrolled from
        // the top, turned in like a page, or slid in. What has not come in
        // yet is cut off; a sliding browser moves.
        let (entrance, _) = variant.theme.panel_entrance();
        let eased = 1.0 - (1.0 - raw).powi(3);
        let dx = if entrance == api::Entrance::Slide { bw * (1.0 - eased) } else { 0.0 };
        let clip = match entrance {
            api::Entrance::Unroll => [bx, by, bw, (bh * eased).max(1.0)],
            api::Entrance::Turn => [bx + bw * (1.0 - eased), by, (bw * eased).max(1.0), bh],
            api::Entrance::Slide => [bx + dx, by, (bw - dx).max(1.0), bh],
        };
        let edge = match entrance {
            api::Entrance::Unroll => by + bh * eased,
            api::Entrance::Turn => bx + bw * (1.0 - eased),
            api::Entrance::Slide => bx + dx,
        };
        let cut = |[x, y, w, h]: [f32; 4]| {
            let (x0, y0) = (x.max(clip[0]), y.max(clip[1]));
            let (x1, y1) = ((x + w).min(clip[0] + clip[2]), (y + h).min(clip[1] + clip[3]));
            (x1 > x0 && y1 > y0).then_some([x0, y0, x1 - x0, y1 - y0])
        };
        let shapes: Vec<Shape> = shapes
            .into_iter()
            .filter_map(|mut shape| {
                shape.rect[0] += dx;
                shape.rect = cut(shape.rect)?;
                Some(shape)
            })
            .collect();
        let mut edges = Vec::new();
        if raw < 1.0 {
            let span = api::PanelSpan { panel: [bx + dx, by, bw, bh], edge, progress: eased };
            variant.theme.draw_panel_edge(variant, &self.ctx(), &span, &mut edges);
        }
        // Clicks only once it is in; while it goes, none.
        look.targets = if raw >= 1.0 { targets } else { Vec::new() };
        look.bounds = if b.closing.is_some() { [0.0; 4] } else { rect };

        // The text goes into the pane's ink layer (`draw` adds it), so the
        // theme's ink stage lays it down as it does the terminal's.
        let bounds = |[x, y, w, h]: [f32; 4]| {
            let [x, y, w, h] = cut([x + dx, y, w, h]).unwrap_or([0.0; 4]);
            TextBounds {
                left: x.max(bx + dx) as i32,
                top: y.max(by) as i32,
                right: (x + w).min(bx + bw + dx) as i32,
                bottom: (y + h).min(by + bh) as i32,
            }
        };
        look.placed = placed.into_iter().map(|(i, x, y, clip)| (i, x + dx, y, bounds(clip))).collect();
        self.browsers.insert(id, look);
        (shapes, edges)
    }
}

/// The folders down to `dir`, each with its name to show: from `~` when it
/// is in the home folder, else from `/`.
fn crumbs(dir: &Path) -> Vec<(String, PathBuf)> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let mut out = Vec::new();
    let mut at = dir.to_path_buf();
    loop {
        if home.as_deref() == Some(at.as_path()) {
            out.push(("~".to_string(), at.clone()));
            break;
        }
        match at.file_name() {
            Some(name) => out.push((name.to_string_lossy().into_owned(), at.clone())),
            None => {
                out.push(("/".to_string(), at.clone()));
                break;
            }
        }
        if !at.pop() {
            break;
        }
    }
    out.reverse();
    out
}
