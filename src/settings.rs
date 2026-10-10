//! The settings panel (Cmd+, or Settings… in the menu): a sheet down the
//! right of the window, so the terminal stays in view and every change can
//! be seen as it is made. It has the settings every theme shares, the
//! current theme's own (from `Theme::params`, so each theme brings its own
//! rows), and the theme and appearance themselves.
//!
//! The panel never keeps settings of its own: every change is written to
//! the settings file (config::apply_edits, keeping the user's comments)
//! and applied from there, and the rows are rebuilt from the loaded
//! settings, so the file and the panel always agree, whichever was edited.
//!
//! This file is the rows and what changing them writes; the renderer draws
//! them (`Renderer::set_settings`) and main.rs routes keys and clicks.

use crate::config::{Appearance, Config, Edit, OptionAsMeta, Value};
use crate::theme::{self, Variant};

/// A row's control.
#[derive(Clone, Debug, PartialEq)]
pub enum Control {
    /// One of a few options, side by side; with more than `SEGMENTS`, one
    /// at a time with arrows either side.
    Choice { labels: Vec<String>, values: Vec<Value>, index: usize },
    Toggle(bool),
    Slider { value: f32, min: f32, max: f32, step: f32 },
}

/// Choices with at most this many options show them all.
pub const SEGMENTS: usize = 3;

#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub label: String,
    pub about: String,
    pub control: Control,
    /// Where it is written: top-level, or a variant's section.
    section: Option<String>,
    key: &'static str,
    /// Font size rows move the line height with them, from (size, line height).
    keeps_spacing: Option<(f32, f32)>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    /// A section's heading. `fold`: None for a section that is always open
    /// (the theme), else whether it is open; a closed one's rows are left
    /// out (see `fold`).
    Heading { title: String, note: String, fold: Option<bool> },
    Row(Row),
}

impl Item {
    pub fn row(&self) -> Option<&Row> {
        match self {
            Item::Row(row) => Some(row),
            Item::Heading { .. } => None,
        }
    }

    /// The row's label or the heading's title: what keeps the keyboard's
    /// place when the items are rebuilt.
    pub fn name(&self) -> &str {
        match self {
            Item::Row(row) => &row.label,
            Item::Heading { title, .. } => title,
        }
    }

    /// A heading that opens and closes.
    pub fn folds(&self) -> bool {
        matches!(self, Item::Heading { fold: Some(_), .. })
    }
}

/// The sections after the first two (the theme, and the look on screen),
/// in order, with the settings in each: what the panel files every other
/// row under.
const SECTIONS: &[(&str, &str, &[&str])] = &[
    ("Animation", "", &["animation", "animation_speed", "writing_speed", "cursor_trail", "cursor_pulse", "paste_splash"]),
    ("Ink and paper", "", &["ink_drying", "drying_time", "ink_aging", "paper_aging", "paper_aging_hours"]),
    ("Effects", "closing, clearing, idle", &["close_effect", "clear_effect", "idle_scene", "idle_after", "show_progress"]),
    ("Commands", "the shell's prompts, marked", &["shell_integration", "mark_failures", "stamp_long_commands", "stamp_after"]),
    ("Sound", "", &["sounds", "sound_volume"]),
    ("Terminal", "", &["harmonize_colors", "grey_backgrounds", "option_as_meta", "strike_deleted", "strike_splatter"]),
];

/// `items` with the sections in `open` open and the others closed: a closed
/// section shows its heading only. The theme's section is always open.
pub fn fold(items: Vec<Item>, open: &std::collections::HashSet<String>) -> Vec<Item> {
    let mut out = Vec::with_capacity(items.len());
    let mut showing = true;
    for item in items {
        match item {
            Item::Heading { title, note, fold: Some(_) } => {
                showing = open.contains(&title);
                out.push(Item::Heading { title, note, fold: Some(showing) });
            }
            heading @ Item::Heading { .. } => {
                showing = true;
                out.push(heading);
            }
            row if showing => out.push(row),
            _ => {}
        }
    }
    out
}

/// The rows after the first two sections, filed under `SECTIONS`.
fn sectioned(items: Vec<Item>) -> Vec<Item> {
    let mut rows: Vec<Row> = Vec::new();
    let mut out = Vec::new();
    let mut headings = 0;
    for item in items {
        match item {
            Item::Heading { title, note, fold } => {
                headings += 1;
                if headings <= 2 {
                    // The look on screen folds; the theme's section doesn't.
                    let fold = if headings == 2 { Some(false) } else { fold };
                    out.push(Item::Heading { title, note, fold });
                }
            }
            Item::Row(row) if headings <= 2 => out.push(Item::Row(row)),
            Item::Row(row) => rows.push(row),
        }
    }
    for (title, note, keys) in SECTIONS {
        out.push(Item::Heading { title: title.to_string(), note: note.to_string(), fold: Some(false) });
        for key in *keys {
            if let Some(i) = rows.iter().position(|r| r.key == *key) {
                out.push(Item::Row(rows.remove(i)));
            }
        }
    }
    // Anything not filed yet goes last, so no setting is ever lost.
    if !rows.is_empty() {
        out.push(Item::Heading { title: "More".into(), note: String::new(), fold: Some(false) });
        out.extend(rows.into_iter().map(Item::Row));
    }
    out
}

/// How a row is changed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Change {
    /// Arrow keys: the next or previous option, or a step of the slider
    /// (`fine`: a small one).
    Step { by: i32, fine: bool },
    /// Return or a click on a toggle: flip it, or the next option.
    Next,
    /// A click on an option.
    Pick(usize),
    /// A click or drag on a slider, this far along it.
    Along(f32),
}

/// What a click landed on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Hit {
    Row(usize, Part),
    /// A section's heading: open or close it.
    Heading(usize),
    /// The button that opens the settings file.
    OpenFile,
}

/// The part of a row a click landed on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Part {
    Label,
    Option(usize),
    Previous,
    Next,
    Toggle,
    /// The slider, with its track's left edge and width.
    Slider(f32, f32),
}

impl Row {
    fn new(label: &str, about: &str, control: Control, section: Option<String>, key: &'static str) -> Self {
        Self { label: label.into(), about: about.into(), control, section, key, keeps_spacing: None }
    }

    /// The settings to write for `change`, or nothing if it changes nothing.
    pub fn change(&self, change: Change) -> Vec<Edit> {
        let edit = |key: &str, value| Edit { section: self.section.clone(), key: key.to_string(), value };
        match &self.control {
            Control::Choice { values, index, .. } => {
                let n = values.len() as i32;
                let to = match change {
                    Change::Step { by, .. } => (*index as i32 + by).rem_euclid(n.max(1)) as usize,
                    Change::Next => (*index + 1) % values.len().max(1),
                    Change::Pick(i) => i,
                    Change::Along(_) => *index,
                };
                match values.get(to) {
                    Some(value) if to != *index => vec![edit(self.key, value.clone())],
                    _ => Vec::new(),
                }
            }
            &Control::Toggle(on) => {
                let to = match change {
                    Change::Step { by, .. } => by > 0,
                    _ => !on,
                };
                if to == on { Vec::new() } else { vec![edit(self.key, Value::Bool(to))] }
            }
            &Control::Slider { value, min, max, step } => {
                let to = match change {
                    Change::Step { by, fine } => {
                        let stride = if fine { step } else { coarse(min, max, step) };
                        value + by as f32 * stride
                    }
                    Change::Along(along) => min + along.clamp(0.0, 1.0) * (max - min),
                    Change::Next | Change::Pick(_) => value,
                };
                let to = snap(to.clamp(min, max), step);
                if (to - value as f64).abs() < step as f64 * 0.5 {
                    return Vec::new();
                }
                let mut edits = vec![edit(self.key, Value::Num(to))];
                if let Some((size, line)) = self.keeps_spacing {
                    edits.push(edit("line_height", Value::Num((line * to as f32 / size).round() as f64)));
                }
                edits
            }
        }
    }

    /// The value as shown beside a slider or between a choice's arrows.
    pub fn value_text(&self) -> String {
        match &self.control {
            Control::Choice { labels, index, .. } => labels.get(*index).cloned().unwrap_or_default(),
            Control::Toggle(on) => if *on { "on" } else { "off" }.into(),
            &Control::Slider { value, step, .. } => {
                let decimals = if step < 0.1 { 2 } else if step < 1.0 { 1 } else { 0 };
                format!("{value:.decimals$}")
            }
        }
    }
}

/// `value` on the slider's grid, as a number that prints cleanly.
fn snap(value: f32, step: f32) -> f64 {
    let steps = (value / step).round() as f64;
    let decimals = (-(step as f64).log10()).ceil().max(0.0) as i32;
    let factor = 10f64.powi(decimals);
    (steps * step as f64 * factor).round() / factor
}

/// A round step about 1/100 of the range, for dragging and Shift+arrows.
fn fine_step(min: f32, max: f32) -> f32 {
    let range = max - min;
    [0.01, 0.02, 0.05, 0.1, 0.2, 0.5, 1.0, 2.0, 5.0, 10.0, 50.0, 100.0]
        .into_iter()
        .find(|&s| s >= range / 100.0)
        .unwrap_or(range / 100.0)
}

/// The arrow keys' step: about 1/20 of the range, on the fine grid.
fn coarse(min: f32, max: f32, step: f32) -> f32 {
    ((max - min) / 20.0 / step).round().max(1.0) * step
}

fn slider(value: f32, min: f32, max: f32) -> Control {
    // A value set in the file beyond the usual range widens the slider.
    let (min, max) = (min.min(value), max.max(value));
    Control::Slider { value, min, max, step: fine_step(min, max) }
}

fn choice(options: &[(&str, &str)], current: &str) -> Control {
    Control::Choice {
        labels: options.iter().map(|(label, _)| label.to_string()).collect(),
        values: options.iter().map(|(_, value)| Value::Str(value.to_string())).collect(),
        index: options.iter().position(|(_, value)| *value == current).unwrap_or(0),
    }
}

/// "bleed_variation" -> "Bleed variation".
fn label(key: &str) -> String {
    let words = key.replace('_', " ");
    let mut chars = words.chars();
    chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default()
}

/// A setting's description as a sentence, without a trailing range
/// ("..., 0..1"), which the slider already shows.
fn describe(about: &str) -> String {
    let text = match about.rsplit_once(", ") {
        Some((head, range)) if range.contains("..") && range.chars().all(|c| c.is_ascii_digit() || c == '.') => head,
        _ => about,
    };
    let mut chars = text.chars();
    let mut sentence: String = chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default();
    if !sentence.ends_with(['.', ')', '!', '?']) {
        sentence.push('.');
    }
    sentence
}

/// What the rows need besides the settings.
pub struct Fonts<'a> {
    /// Installed monospace families, sorted.
    pub families: &'a [String],
    /// Weights the current variant's family has.
    pub weights: &'a [u16],
}

/// The panel's rows for `config`, showing `variant` (the one on screen).
pub fn items(config: &Config, variant: &Variant, fonts: &Fonts) -> Vec<Item> {
    let heading = |title: &str, note: &str| Item::Heading { title: title.into(), note: note.into(), fold: None };
    let top = |label: &str, about: &str, control, key| Item::Row(Row::new(label, about, control, None, key));
    let mut items = vec![heading("Theme", "")];

    let themes = theme::all();
    let ids: Vec<(&str, &str)> = themes.iter().map(|t| (t.name(), t.id())).collect();
    items.push(top("Theme", "Cmd+Option+T shows them all side by side.", choice(&ids, variant.theme.id()), "theme"));
    let appearance = match config.appearance {
        Appearance::System => "system",
        Appearance::Light => "light",
        Appearance::Dark => "dark",
    };
    items.push(top(
        "Appearance",
        "System follows macOS. Cmd+Shift+L flips it for now.",
        choice(&[("System", "system"), ("Light", "light"), ("Dark", "dark")], appearance),
        "appearance",
    ));

    // The variant on screen: its fonts, then its theme's own settings.
    let section = Some(variant.key());
    let own = |label: &str, about: &str, control, key| Item::Row(Row::new(label, about, control, section.clone(), key));
    items.push(heading(variant.name, "this look only; Cmd+Shift+L shows the other"));
    let mut families = fonts.families.to_vec();
    if !families.iter().any(|f| f.eq_ignore_ascii_case(&variant.font_family)) {
        families.insert(0, variant.font_family.clone());
    }
    let family_index = families.iter().position(|f| f.eq_ignore_ascii_case(&variant.font_family)).unwrap_or(0);
    items.push(own(
        "Font",
        "Installed monospace fonts.",
        Control::Choice {
            values: families.iter().map(|f| Value::Str(f.clone())).collect(),
            labels: families,
            index: family_index,
        },
        "font_family",
    ));
    let mut size = Row::new("Size", "", slider(variant.font_size, 9.0, 32.0), section.clone(), "font_size");
    if let Control::Slider { step, .. } = &mut size.control {
        *step = 0.5;
    }
    size.keeps_spacing = Some((variant.font_size, variant.line_height));
    items.push(Item::Row(size));
    let mut weights = fonts.weights.to_vec();
    if weights.is_empty() {
        weights = vec![variant.font_weight];
    }
    let weight_index = weights
        .iter()
        .enumerate()
        .min_by_key(|(_, w)| w.abs_diff(variant.font_weight))
        .map_or(0, |(i, _)| i);
    items.push(own(
        "Weight",
        "The weights this font has.",
        Control::Choice {
            labels: weights.iter().map(|w| weight_name(*w)).collect(),
            values: weights.iter().map(|&w| Value::Num(w as f64)).collect(),
            index: weight_index,
        },
        "font_weight",
    ));
    let mut line = Row::new(
        "Line height",
        "Pixels from one line to the next.",
        slider(variant.line_height, (variant.font_size * 1.0).floor(), (variant.font_size * 2.4).ceil()),
        section.clone(),
        "line_height",
    );
    if let Control::Slider { step, .. } = &mut line.control {
        *step = 1.0;
    }
    items.push(Item::Row(line));
    for (param, &value) in variant.theme.params().iter().zip(&variant.params) {
        items.push(Item::Row(Row::new(
            &label(param.key),
            &describe(param.about),
            slider(value, param.min, param.max),
            section.clone(),
            param.key,
        )));
    }

    let (animation, animation_speed, writing_speed) = config.animation();
    items.push(heading("Animation", ""));
    items.push(top(
        "Animation",
        "Writing new characters in, and the theme's reveals and switches.",
        Control::Toggle(animation),
        "animation",
    ));
    items.push(top("Speed", "2 is twice as fast, 0.5 half speed.", slider(animation_speed, 0.25, 4.0), "animation_speed"));
    let mut writing = Row::new(
        "Writing speed",
        "Characters per second as output is written out.",
        slider(writing_speed, 50.0, 3000.0),
        None,
        "writing_speed",
    );
    if let Control::Slider { step, .. } = &mut writing.control {
        *step = 10.0;
    }
    items.push(Item::Row(writing));
    items.push(top(
        "Ink dries",
        "Fresh ink looks wet, then dries: darker and glossy, or brighter chalk.",
        Control::Toggle(config.ink_drying.unwrap_or(true)),
        "ink_drying",
    ));
    items.push(top("Drying time", "Seconds fresh ink stays wet.", slider(config.drying_time.unwrap_or(4.0), 1.0, 20.0), "drying_time"));
    items.push(top(
        "Cursor trail",
        "The cursor flicks a trail of ink behind it when it jumps.",
        Control::Toggle(config.cursor_trail.unwrap_or(true)),
        "cursor_trail",
    ));
    items.push(top(
        "Cursor heartbeat",
        "A still cursor beats gently, so it is easy to find.",
        Control::Toggle(config.cursor_pulse.unwrap_or(true)),
        "cursor_pulse",
    ));
    items.push(top(
        "Paste splash",
        "Pasted text lands at once, as a splash of ink that settles into letters.",
        Control::Toggle(config.paste_splash.unwrap_or(true)),
        "paste_splash",
    ));
    items.push(top(
        "Show progress",
        "Programs that report progress get a stroke of ink along the title strip.",
        Control::Toggle(config.show_progress.unwrap_or(true)),
        "show_progress",
    ));
    items.push(top(
        "Sounds",
        "A brush as you type, the sword's shing, a seal's thud; chalk on the board.",
        Control::Toggle(config.sounds.unwrap_or(false)),
        "sounds",
    ));
    items.push(top("Volume", "How loud the sounds are.", slider(config.sound_volume.unwrap_or(0.4), 0.0, 1.0), "sound_volume"));
    items.push(top(
        "Paper ages",
        "A tab's sheet yellows and creases as it is used; a new tab is fresh.",
        Control::Toggle(config.paper_aging.unwrap_or(true)),
        "paper_aging",
    ));
    let mut hours = Row::new("Aging hours", "Hours of use for a sheet to wear out.", slider(config.paper_aging_hours.unwrap_or(8.0), 1.0, 48.0), None, "paper_aging_hours");
    if let Control::Slider { step, .. } = &mut hours.control {
        *step = 1.0;
    }
    items.push(Item::Row(hours));
    items.push(top(
        "Closing",
        "Closed tabs, windows and panes burn, tear or slide away, the theme's way.",
        Control::Toggle(config.close_effect.unwrap_or(true)),
        "close_effect",
    ));
    items.push(top(
        "Clearing",
        "Cmd+K or `clear` burns the old screen away from the cursor, or blows the chalk off.",
        Control::Toggle(config.clear_effect.unwrap_or(true)),
        "clear_effect",
    ));
    items.push(top(
        "Idle scene",
        "Left alone, the window gets the theme's scene: falling leaves, settling dust.",
        Control::Toggle(config.idle_scene.unwrap_or(true)),
        "idle_scene",
    ));
    let mut idle = Row::new("Idle after", "Seconds without a key or a click.", slider(config.idle_after.unwrap_or(60.0), 10.0, 600.0), None, "idle_after");
    if let Control::Slider { step, .. } = &mut idle.control {
        *step = 5.0;
    }
    items.push(Item::Row(idle));
    items.push(top(
        "Ink aging",
        "How far ink an hour old has faded; 0 never.",
        slider(config.ink_aging.unwrap_or(0.3), 0.0, 1.0),
        "ink_aging",
    ));

    items.push(heading("Terminal", ""));
    items.push(top(
        "Harmonise colours",
        "Turn colours programs pick for a black screen into inks that read on this one.",
        Control::Toggle(config.harmonize_colors.unwrap_or(true)),
        "harmonize_colors",
    ));
    items.push(top(
        "Grey backgrounds",
        "Show the grey bands programs draw behind text (Claude Code's messages).",
        Control::Toggle(config.grey_backgrounds.unwrap_or(false)),
        "grey_backgrounds",
    ));
    let meta = match config.option_as_meta {
        OptionAsMeta::Left => "left",
        OptionAsMeta::Both => "both",
        OptionAsMeta::None => "none",
    };
    items.push(top(
        "Option as Meta",
        "Which Option key sends Alt+B, Alt+F, ...; the other still types é, €.",
        choice(&[("Left", "left"), ("Both", "both"), ("None", "none")], meta),
        "option_as_meta",
    ));
    items.push(top(
        "Strike deleted",
        "Strike through names of files and folders deleted while on screen.",
        Control::Toggle(config.strike_deleted.unwrap_or(true)),
        "strike_deleted",
    ));
    items.push(top(
        "Strike extras",
        "What the theme adds to a strike: Tsushima's blood, Chalkboard's dust.",
        Control::Toggle(config.strike_splatter.unwrap_or(true)),
        "strike_splatter",
    ));

    items.push(heading("Commands", "the shell's prompts, marked"));
    items.push(top(
        "Shell integration",
        "The shell marks its prompts, so commands can be marked. New shells only.",
        Control::Toggle(config.shell_integration.unwrap_or(true)),
        "shell_integration",
    ));
    items.push(top(
        "Mark failures",
        "A command that fails gets the theme's mark beside its prompt.",
        Control::Toggle(config.mark_failures.unwrap_or(true)),
        "mark_failures",
    ));
    items.push(top(
        "Stamp long commands",
        "A command that ran a while is stamped when it finishes.",
        Control::Toggle(config.stamp_long_commands.unwrap_or(true)),
        "stamp_long_commands",
    ));
    let mut after = Row::new(
        "Stamp after",
        "Seconds a command must run to be stamped.",
        slider(config.stamp_after.unwrap_or(10.0), 2.0, 120.0),
        None,
        "stamp_after",
    );
    if let Control::Slider { step, .. } = &mut after.control {
        *step = 1.0;
    }
    items.push(Item::Row(after));
    sectioned(items)
}

/// "400" -> "Regular" and so on, as font menus name them.
fn weight_name(weight: u16) -> String {
    match weight {
        100 => "Thin",
        200 => "Extra Light",
        300 => "Light",
        400 => "Regular",
        500 => "Medium",
        600 => "Semibold",
        700 => "Bold",
        800 => "Heavy",
        900 => "Black",
        _ => return weight.to_string(),
    }
    .into()
}

/// The row after (or before, `by` < 0) `from`, skipping headings; `from`
/// itself when there is none.
pub fn next_row(items: &[Item], from: usize, by: i32) -> usize {
    let mut i = from as i32;
    loop {
        i += by.signum();
        match items.get(i.max(0) as usize) {
            _ if i < 0 => return from,
            Some(Item::Row(_)) => return i as usize,
            Some(item) if item.folds() => return i as usize,
            Some(Item::Heading { .. }) => continue,
            None => return from,
        }
    }
}

/// The first row.
pub fn first_row(items: &[Item]) -> usize {
    items.iter().position(|i| i.row().is_some()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(config: &Config, variant: &Variant) -> Vec<Item> {
        let families = vec!["Courier New".to_string(), "Menlo".to_string()];
        items(config, variant, &Fonts { families: &families, weights: &[400, 700] })
    }

    fn find<'a>(items: &'a [Item], label: &str) -> &'a Row {
        items.iter().filter_map(Item::row).find(|r| r.label == label).unwrap()
    }

    /// Applies a row's change to `text`, as the panel does.
    fn change(text: &str, items: &[Item], label: &str, change: Change) -> String {
        crate::config::apply_edits(text, &find(items, label).change(change)).unwrap()
    }

    #[test]
    fn every_theme_brings_its_own_rows() {
        let config = Config::default();
        for variant in theme::builtin() {
            let items = rows(&config, &variant);
            for param in variant.theme.params() {
                find(&items, &label(param.key));
            }
            assert!(items.iter().any(|i| matches!(i, Item::Heading { title, .. } if title == variant.name)));
        }
    }

    #[test]
    fn changes_reach_the_file_and_come_back() {
        let variants = theme::builtin();
        let light = variants.iter().find(|v| v.key() == "tsushima-light").unwrap();
        let mut text = crate::config::template();
        let items = rows(&Config::parse(&text).unwrap(), light);
        text = change(&text, &items, "Nib", Change::Along(0.5));
        text = change(&text, &items, "Animation", Change::Next);
        text = change(&text, &items, "Appearance", Change::Pick(2));
        text = change(&text, &items, "Speed", Change::Step { by: 1, fine: false });
        text = change(&text, &items, "Theme", Change::Step { by: 1, fine: false });
        let config = Config::parse(&text).unwrap();
        let light = config.themes().into_iter().find(|v| v.key() == "tsushima-light").unwrap();
        assert_eq!(light.param("nib"), 5.0);
        assert_eq!(config.animation().0, false);
        assert_eq!(config.appearance, Appearance::Dark);
        assert_eq!(config.theme.as_deref(), Some("chalkboard"));
        // The rows built from the result show what was written.
        let again = rows(&config, &light);
        assert_eq!(find(&again, "Nib").value_text(), "5.0");
        assert_eq!(find(&again, "Animation").control, Control::Toggle(false));
        assert!(find(&again, "Speed").value_text().starts_with("1.2"));
    }

    #[test]
    fn font_size_keeps_the_spacing() {
        let variants = theme::builtin();
        let light = &variants[0];
        let text = crate::config::template();
        let items = rows(&Config::parse(&text).unwrap(), light);
        let size = light.font_size;
        let text = change(&text, &items, "Size", Change::Along(1.0));
        let after = Config::parse(&text).unwrap().themes().into_iter().find(|v| v.name == light.name).unwrap();
        assert_eq!(after.font_size, 32.0);
        assert_eq!(after.line_height, (light.line_height * 32.0 / size).round());
    }

    #[test]
    fn sliders_stay_in_range_and_on_their_grid() {
        let row = Row::new("x", "", slider(0.3, 0.0, 1.0), None, "x");
        assert!(row.change(Change::Step { by: -1, fine: true }) == vec![Edit { section: None, key: "x".into(), value: Value::Num(0.29) }]);
        assert_eq!(row.change(Change::Along(3.0))[0].value, Value::Num(1.0));
        assert!(Row::new("x", "", slider(0.0, 0.0, 1.0), None, "x").change(Change::Step { by: -1, fine: false }).is_empty());
        assert_eq!(describe("dry-brush streaks, 0..1"), "Dry-brush streaks.");
        assert_eq!(describe("water-stain tide lines: 1 built-in, 0 none, up to 3"), "Water-stain tide lines: 1 built-in, 0 none, up to 3.");
    }

    #[test]
    fn keys_move_between_rows_and_section_headings() {
        let items = rows(&Config::default(), &theme::builtin()[0]);
        let first = first_row(&items);
        assert_eq!(next_row(&items, first, -1), first);
        let mut i = first;
        let mut seen = 1;
        loop {
            let next = next_row(&items, i, 1);
            if next == i {
                break;
            }
            // The keyboard stops on rows and on headings that fold, never
            // on the theme's own heading.
            assert!(items[next].row().is_some() || items[next].folds());
            (i, seen) = (next, seen + 1);
        }
        assert_eq!(seen, items.iter().filter(|i| i.row().is_some() || i.folds()).count());
    }

    #[test]
    fn sections_fold_and_every_setting_is_filed() {
        let items = rows(&Config::default(), &theme::builtin()[0]);
        // Every setting has a section of its own, not the catch-all.
        assert!(!items.iter().any(|i| i.name() == "More"), "a setting is not in SECTIONS");
        let closed = fold(items.clone(), &std::collections::HashSet::new());
        // Closed, only the theme's rows and the section headings are left.
        let shown: Vec<&str> = closed.iter().map(Item::name).collect();
        assert!(shown.contains(&"Theme") && shown.contains(&"Appearance") && shown.contains(&"Sound"), "{shown:?}");
        assert!(!shown.contains(&"Volume"));
        let open: std::collections::HashSet<String> = ["Sound".to_string()].into();
        let opened = fold(items, &open);
        assert!(opened.iter().any(|i| i.name() == "Volume"));
        assert!(opened.iter().any(|i| matches!(i, Item::Heading { title, fold: Some(true), .. } if title == "Sound")));
    }
}
