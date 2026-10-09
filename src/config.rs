//! Settings file: `~/.config/oneprompt/config.toml` (or `$XDG_CONFIG_HOME`).
//!
//! Cmd+, opens it in the default editor (creating it from `template()` first),
//! and a watcher thread reloads it whenever it is saved, so every setting
//! applies live. Everything is optional; a missing file means defaults.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde::Deserialize;
use winit::event_loop::EventLoopProxy;

use crate::terminal::UserEvent;
use crate::theme::Theme;

/// Which Option key sends Meta (ESC + key) instead of composing characters.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum OptionAsMeta {
    /// Left Option is Meta, right Option still types é, €, ...
    #[default]
    Left,
    Both,
    None,
}

/// Light or dark variant of the theme.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Appearance {
    /// Follow macOS light / dark mode, switching live when it changes.
    #[default]
    System,
    Light,
    Dark,
}

/// Per-theme overrides: `[themes.tsushima-light]` and so on.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct ThemeOverride {
    pub font_family: Option<String>,
    pub font_size: Option<f32>,
    pub font_weight: Option<u16>,
    pub line_height: Option<f32>,
    pub bleed: Option<f32>,
    pub grain: Option<f32>,
    pub edge: Option<f32>,
    pub nib: Option<f32>,
    /// Per-character spread of the bleed, 0..1: 0 every letter alike.
    pub bleed_variation: Option<f32>,
    /// Seconds the bleed keeps spreading after a letter is written.
    pub bleed_time: Option<f32>,
    /// How much the ink takes on the paper's grain, 0..1.
    pub absorb: Option<f32>,
    /// Paper texture strength (mottling, relief, fibers): 1 built-in, 0 flat.
    pub texture: Option<f32>,
    /// Water-stain strength: 1 built-in, 0 none.
    pub stains: Option<f32>,
    /// Ink splatter in the corner: 1 built-in, 0 none.
    pub splatter: Option<f32>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Light or dark variant; Cmd+Shift+L flips it for the session.
    pub appearance: Appearance,
    /// Pin the paper pattern; unset gives a new sheet every session.
    pub seed: Option<f32>,
    pub option_as_meta: OptionAsMeta,
    /// Ink animations: characters brushed in, window reveals. Default on.
    pub animation: Option<bool>,
    /// 1 = normal, 2 = twice as fast, 0.5 = half speed.
    pub animation_speed: Option<f32>,
    /// Characters per second when output is written out line by line.
    pub writing_speed: Option<f32>,
    /// Map colours programs pick themselves (truecolor, 256-colour) onto the
    /// sheet as inks. Default on.
    pub harmonize_colors: Option<bool>,
    /// Font settings for every theme (per-theme sections win).
    pub font_family: Option<String>,
    pub font_size: Option<f32>,
    pub font_weight: Option<u16>,
    pub themes: HashMap<String, ThemeOverride>,
}

/// The file written the first time settings are opened. Settings are
/// active lines holding the built-in values, so changing a number just works
/// (an all-commented template invites editing a line that stays a comment).
/// Generated from the themes, so it never drifts from the defaults.
pub fn template() -> String {
    let themes = crate::theme::builtin();
    let mut out = String::from(
        r#"# oneprompt settings. Save the file and changes apply immediately.
# Lines starting with # are comments and are ignored.

# Light or dark: "system" follows macOS and switches with it; "light" or
# "dark" pins one. Cmd+Shift+L flips between them for the session.
appearance = "system"

# Paper pattern. Unset: a new sheet every session. A number pins one sheet.
# seed = 11

# Which Option key acts as Meta (Alt+B, Alt+F, ...): "left", "both" or "none".
option_as_meta = "left"

# Ink animations: new characters are brushed in and dry, the window is
# revealed with a brush when it opens and when light / dark switches.
animation = true
animation_speed = 1.0        # 2 = twice as fast, 0.5 = half speed
# Output (ls, cat, ...) is written left to right, line by line, at this many
# characters per second. Long output is sped up to finish within 1.5 seconds.
writing_speed = 500

# Programs that pick exact colours (Claude Code, bat, delta, ...) choose them
# for a plain black screen. This keeps their hues but turns them into inks
# that read on the sheet. false shows them exactly as sent.
harmonize_colors = true

# The light and dark variants, set to their built-in values; change any of them.
# Fonts must be monospace. font_weight snaps to the weights the font has
# (Courier New and PT Mono: 400 regular, 700 bold).
#   bleed  how far ink wicks past strokes (pixels)
#   bleed_variation  how much the bleed differs per letter, 0..1 (0.35: each
#                    letter bleeds 0.65 to 1.35 times `bleed`)
#   bleed_time       seconds the bleed keeps spreading after a letter is written
#   nib    brush nib length: thick-and-thin strokes (pixels, 0 = off)
#   grain  dry-brush streaks, 0..1
#   edge   pigment pooling at stroke rims, 0..1
#   absorb how much the ink takes on the paper's grain, 0..1 (0 = ink sits on top)
#   texture   paper texture (mottling, relief, fibers): 1 built-in, 0 flat, up to 3
#   stains    water-stain tide lines: 1 built-in, 0 none, up to 3
#   splatter  ink flecks in the top-right corner: 1 built-in, 0 none, up to 3
"#,
    );
    for theme in &themes {
        let ink = theme.ink_style;
        out += &format!(
            "\n[themes.{key}]\nfont_family = \"{family}\"\nfont_size = {size}\nfont_weight = {weight}\n\
             line_height = {line}\nbleed = {bleed}\nbleed_variation = {variation}\nbleed_time = {bleed_time}\nnib = {nib}\ngrain = {grain}\nedge = {edge}\nabsorb = {absorb}\n\
             texture = {texture}\nstains = {stains}\nsplatter = {splatter}\n",
            key = key(theme.name),
            family = theme.font_family,
            size = theme.font_size,
            weight = theme.font_weight,
            line = theme.line_height,
            bleed = ink.bleed,
            variation = ink.bleed_variation,
            bleed_time = ink.bleed_time,
            nib = ink.nib,
            grain = ink.grain,
            edge = ink.edge,
            absorb = ink.absorb,
            texture = theme.texture_strength,
            stains = theme.stains,
            splatter = theme.splatter.1,
        );
    }
    out
}

/// Settings that belong to a variant, under `[themes.<name>]`.
const THEME_SETTINGS: [&str; 14] = [
    "font_family",
    "font_size",
    "font_weight",
    "line_height",
    "bleed",
    "bleed_variation",
    "bleed_time",
    "absorb",
    "nib",
    "grain",
    "edge",
    "texture",
    "stains",
    "splatter",
];

/// A parse error in words that say how to fix it. The most common mistake
/// is a per-theme setting outside its section, which toml reports as an
/// unknown field followed by the whole list of valid ones.
fn explain(message: &str) -> String {
    let Some(name) = message.strip_prefix("unknown field `").and_then(|rest| rest.split('`').next()) else {
        return message.trim().to_string();
    };
    if THEME_SETTINGS.contains(&name) {
        let sections: Vec<String> = crate::theme::builtin().iter().map(|t| format!("[themes.{}]", key(t.name))).collect();
        return format!("`{name}` is set per theme: put it under {}", sections.join(" or "));
    }
    format!("unknown setting `{name}`")
}

/// `~/.config/oneprompt/config.toml`, honouring `$XDG_CONFIG_HOME`.
/// `ONEPROMPT_CONFIG` points somewhere else (tests, screenshots).
pub fn path() -> PathBuf {
    if let Some(path) = std::env::var_os("ONEPROMPT_CONFIG") {
        return PathBuf::from(path);
    }
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("oneprompt").join("config.toml")
}

/// Theme names as they appear in section headers: "Tsushima Dark" -> "tsushima-dark".
pub fn key(name: &str) -> String {
    name.trim().to_lowercase().replace([' ', '_'], "-")
}

impl Config {
    pub fn parse(text: &str) -> Result<Self, String> {
        let config: Config = toml::from_str(text).map_err(|e| explain(e.message()))?;
        config.validate()?;
        Ok(config)
    }

    /// Loads the file; a missing file is the default config.
    pub fn load(path: &Path) -> Result<Self, String> {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::parse(&text),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(err) => Err(err.to_string()),
        }
    }

    fn validate(&self) -> Result<(), String> {
        let names: Vec<String> = crate::theme::builtin().iter().map(|t| key(t.name)).collect();
        if let Some(name) = self.themes.keys().find(|k| !names.contains(&key(k))) {
            return Err(format!("unknown theme section [themes.{name}]"));
        }
        let sizes = std::iter::once(self.font_size)
            .chain(self.themes.values().map(|o| o.font_size))
            .flatten();
        if let Some(size) = sizes.into_iter().find(|s| !(4.0..=96.0).contains(s)) {
            return Err(format!("font_size {size} is out of range (4 to 96)"));
        }
        let paper = self.themes.values().flat_map(|o| [o.texture, o.stains, o.splatter]).flatten();
        if let Some(v) = paper.into_iter().find(|v| !(0.0..=3.0).contains(v)) {
            return Err(format!("texture / stains / splatter {v} is out of range (0 to 3)"));
        }
        if let Some(v) = self.themes.values().find_map(|o| o.bleed_variation.filter(|v| !(0.0..=1.0).contains(v))) {
            return Err(format!("bleed_variation {v} is out of range (0 to 1)"));
        }
        if let Some(v) = self.themes.values().find_map(|o| o.absorb.filter(|v| !(0.0..=1.0).contains(v))) {
            return Err(format!("absorb {v} is out of range (0 to 1)"));
        }
        if let Some(v) = self.themes.values().find_map(|o| o.bleed_time.filter(|v| !(0.0..=30.0).contains(v))) {
            return Err(format!("bleed_time {v} is out of range (0 to 30 seconds)"));
        }
        if let Some(speed) = self.animation_speed
            && !(0.1..=10.0).contains(&speed)
        {
            return Err(format!("animation_speed {speed} is out of range (0.1 to 10)"));
        }
        if let Some(speed) = self.writing_speed
            && !(10.0..=100_000.0).contains(&speed)
        {
            return Err(format!("writing_speed {speed} is out of range (10 to 100000)"));
        }
        Ok(())
    }

    /// Animations on, their speed, and the writing speed (characters/s).
    pub fn animation(&self) -> (bool, f32, f32) {
        (
            self.animation.unwrap_or(true),
            self.animation_speed.unwrap_or(1.0),
            self.writing_speed.unwrap_or(500.0),
        )
    }

    /// The built-in themes with this config's overrides applied.
    pub fn themes(&self) -> Vec<Theme> {
        let mut themes = crate::theme::builtin();
        for theme in &mut themes {
            apply_fonts(theme, &self.font_family, self.font_size, self.font_weight);
            let Some(o) = self.themes.iter().find(|(k, _)| key(k) == key(theme.name)).map(|(_, o)| o)
            else {
                continue;
            };
            apply_fonts(theme, &o.font_family, o.font_size, o.font_weight);
            if let Some(v) = o.line_height {
                theme.line_height = v;
            }
            theme.texture_strength = o.texture.unwrap_or(theme.texture_strength);
            theme.stains = o.stains.unwrap_or(theme.stains);
            theme.splatter.1 = o.splatter.unwrap_or(theme.splatter.1);
            let ink = &mut theme.ink_style;
            ink.bleed = o.bleed.unwrap_or(ink.bleed);
            ink.grain = o.grain.unwrap_or(ink.grain);
            ink.edge = o.edge.unwrap_or(ink.edge);
            ink.nib = o.nib.unwrap_or(ink.nib);
            ink.bleed_variation = o.bleed_variation.unwrap_or(ink.bleed_variation);
            ink.bleed_time = o.bleed_time.unwrap_or(ink.bleed_time);
            ink.absorb = o.absorb.unwrap_or(ink.absorb);
        }
        themes
    }

    /// Index of the theme to show, given whether the system is in dark mode
    /// and any session override from Cmd+Shift+L.
    pub fn theme_index(&self, themes: &[Theme], system_dark: bool, flip: Option<bool>) -> usize {
        let dark = flip.unwrap_or(match self.appearance {
            Appearance::System => system_dark,
            Appearance::Light => false,
            Appearance::Dark => true,
        });
        themes.iter().position(|t| t.dark == dark).unwrap_or(0)
    }
}

fn apply_fonts(theme: &mut Theme, family: &Option<String>, size: Option<f32>, weight: Option<u16>) {
    if let Some(family) = family {
        theme.font_family = family.clone();
    }
    if let Some(size) = size {
        // Keep the theme's line spacing proportional to the new size.
        theme.line_height = (theme.line_height * size / theme.font_size).round();
        theme.font_size = size;
    }
    if let Some(weight) = weight {
        theme.font_weight = weight.clamp(100, 900);
    }
}

/// Creates the settings file from the template if needed, then opens it in
/// the system's default text editor.
pub fn open_in_editor(path: &Path) -> std::io::Result<()> {
    if !path.exists() {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, template())?;
    }
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut c = std::process::Command::new("open");
        c.arg("-t");
        c
    };
    #[cfg(not(target_os = "macos"))]
    let mut command = std::process::Command::new("xdg-open");
    command.arg(path).spawn().map(|_| ())
}

/// Polls the file's modification time and sends `ConfigChanged` when it
/// changes (also when it appears or is deleted).
pub fn watch(path: PathBuf, proxy: EventLoopProxy<UserEvent>) {
    std::thread::Builder::new()
        .name("config watcher".into())
        .spawn(move || {
            let stamp = |p: &Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
            let mut last: Option<SystemTime> = stamp(&path);
            loop {
                std::thread::sleep(Duration::from_millis(400));
                let now = stamp(&path);
                if now != last {
                    last = now;
                    if proxy.send_event(UserEvent::ConfigChanged).is_err() {
                        return;
                    }
                }
            }
        })
        .expect("spawn config watcher");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_holds_exactly_the_builtin_look() {
        let config = Config::parse(&template()).unwrap();
        let (ours, builtin) = (config.themes(), crate::theme::builtin());
        assert_eq!(ours.len(), builtin.len());
        for (a, b) in ours.iter().zip(&builtin) {
            assert_eq!(
                (&a.font_family, a.font_size, a.font_weight, a.line_height),
                (&b.font_family, b.font_size, b.font_weight, b.line_height),
                "{}",
                a.name
            );
            let (x, y) = (a.ink_style, b.ink_style);
            assert_eq!((x.bleed, x.nib, x.grain, x.edge), (y.bleed, y.nib, y.grain, y.edge));
        }
        assert_eq!(config.appearance, Appearance::System);
    }

    #[test]
    fn paper_strengths_are_settable() {
        let config = Config::parse("[themes.tsushima-light]\ntexture = 0.5\nstains = 0\nsplatter = 2").unwrap();
        let light = config.themes().into_iter().find(|t| !t.dark).unwrap();
        assert_eq!((light.texture_strength, light.stains, light.splatter.1), (0.5, 0.0, 2.0));
        assert!(Config::parse("[themes.tsushima-light]\nstains = 5").unwrap_err().contains("out of range"));
    }

    #[test]
    fn misplaced_theme_settings_say_where_they_go() {
        let err = Config::parse("bleed = 1.7").unwrap_err();
        assert!(err.contains("[themes.tsushima-light]") && err.len() < 120, "{err}");
        assert_eq!(Config::parse("blead = 1").unwrap_err(), "unknown setting `blead`");
    }

    #[test]
    fn bleed_animation_is_settable() {
        let config = Config::parse("[themes.tsushima-light]\nbleed_variation = 0.6\nbleed_time = 4").unwrap();
        let light = config.themes().into_iter().find(|t| !t.dark).unwrap();
        assert_eq!((light.ink_style.bleed_variation, light.ink_style.bleed_time), (0.6, 4.0));
        assert!(Config::parse("[themes.tsushima-light]\nbleed_variation = 2").is_err());
        assert!(Config::parse("[themes.tsushima-light]\nbleed_time = -1").is_err());
    }

    #[test]
    fn editing_a_template_value_takes_effect() {
        let edited = template().replacen("font_weight = 700", "font_weight = 400", 1);
        let themes = Config::parse(&edited).unwrap().themes();
        assert_eq!(themes[0].font_weight, 400);
    }

    #[test]
    fn global_and_per_theme_overrides() {
        let config = Config::parse(
            r#"
            font_size = 20
            [themes.tsushima-light]
            nib = 0.5
            font_family = "Menlo"
            "#,
        )
        .unwrap();
        let themes = config.themes();
        let light = themes.iter().find(|t| !t.dark).unwrap();
        assert_eq!(light.font_size, 20.0);
        assert_eq!(light.font_family, "Menlo");
        assert_eq!(light.ink_style.nib, 0.5);
        // Line height scales with the size: 24 at 17pt becomes 28 at 20pt.
        assert_eq!(light.line_height, 28.0);
        // The global size reaches the dark variant; the light-only font does not.
        let dark = themes.iter().find(|t| t.dark).unwrap();
        assert_eq!(dark.font_size, 20.0);
        assert_eq!(dark.font_family, "Courier New");
    }

    #[test]
    fn appearance_picks_the_variant() {
        let themes = crate::theme::builtin();
        let pick = |text: &str, system_dark, flip| {
            let config = Config::parse(text).unwrap();
            themes[config.theme_index(&themes, system_dark, flip)].dark
        };
        assert!(!pick("", false, None)); // follows the system...
        assert!(pick("", true, None));
        assert!(!pick("appearance = \"light\"", true, None)); // ...unless pinned
        assert!(pick("appearance = \"dark\"", false, None));
        assert!(pick("appearance = \"light\"", false, Some(true))); // Cmd+Shift+L wins
    }

    #[test]
    fn mistakes_are_reported() {
        assert!(Config::parse("appearance = \"dusk\"").is_err());
        assert!(Config::parse("[themes.nope]\nnib = 1").unwrap_err().contains("[themes.nope]"));
        assert!(Config::parse("font_size = 500").unwrap_err().contains("out of range"));
        assert!(Config::parse("colour = 1").is_err());
        assert!(Config::parse("[themes.tsushima-dark]\nnibb = 1").is_err());
        assert!(Config::parse("option_as_meta = \"both\"").is_ok());
        assert!(Config::parse("animation_speed = 0").unwrap_err().contains("out of range"));
        assert_eq!(Config::parse("animation = false").unwrap().animation(), (false, 1.0, 500.0));
        assert_eq!(Config::parse("writing_speed = 120").unwrap().animation().2, 120.0);
        assert!(Config::parse("writing_speed = 1").unwrap_err().contains("out of range"));
    }

    #[test]
    fn theme_keys_ignore_case_and_spaces() {
        assert_eq!(key("Tsushima Dark"), "tsushima-dark");
        assert_eq!(key(" tsushima_dark "), "tsushima-dark");
    }
}
