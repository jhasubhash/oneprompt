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
use crate::theme::{self, Variant, key};

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

/// Per-variant overrides: `[themes.tsushima-light]` and so on. Fonts are
/// the same for every theme; the rest are the theme's own settings
/// (`Theme::params`), checked against what it declares.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct ThemeOverride {
    pub font_family: Option<String>,
    pub font_size: Option<f32>,
    pub font_weight: Option<u16>,
    pub line_height: Option<f32>,
    #[serde(flatten)]
    pub params: HashMap<String, f32>,
}

/// Settings every variant has, whatever its theme.
const FONT_SETTINGS: [&str; 4] = ["font_family", "font_size", "font_weight", "line_height"];

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Which theme (`Theme::id`); unset is the first one.
    pub theme: Option<String>,
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
    /// Strike out names of files and folders deleted while on screen.
    /// Default on.
    pub strike_deleted: Option<bool>,
    /// Extras on top of the strike (Tsushima: blood flung off the sword).
    /// Default on.
    pub strike_splatter: Option<bool>,
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
    let themes = theme::all();
    let ids: Vec<String> = themes.iter().map(|t| format!("\"{}\"", t.id())).collect();
    let mut out = format!(
        r#"# oneprompt settings. Save the file and changes apply immediately.
# Lines starting with # are comments and are ignored.

# Which theme: {}. Cmd+Option+T opens the theme picker.
theme = "{}"

# Light or dark: "system" follows macOS and switches with it; "light" or
# "dark" pins one. Cmd+Shift+L flips between them for the session.
appearance = "system"

# Paper pattern. Unset: a new sheet every session. A number pins one sheet.
# seed = 11

# Which Option key acts as Meta (Alt+B, Alt+F, ...): "left", "both" or "none".
option_as_meta = "left"

# Animations: new characters are written in by hand, and the theme reveals
# the window when it opens and when the look switches.
animation = true
animation_speed = 1.0        # 2 = twice as fast, 0.5 = half speed
# Output (ls, cat, ...) is written left to right, line by line, at this many
# characters per second. Long output is sped up to finish within 1.5 seconds.
writing_speed = 500

# Programs that pick exact colours (Claude Code, bat, delta, ...) choose them
# for a plain black screen. This keeps their hues but turns them into inks
# that read on the sheet. false shows them exactly as sent.
harmonize_colors = true

# When a file or folder named on screen is deleted (rm, a script, the
# Finder, ...), every mention of it is struck through, as the theme does it.
strike_deleted = true
strike_splatter = true       # extras: Tsushima's blood; false: a clean strike

# Each theme's variants, set to their built-in values; change any of them.
# Fonts must be monospace. font_weight snaps to the weights the font has
# (Courier New and PT Mono: 400 regular, 700 bold).
"#,
        ids.join(", "),
        themes[0].id(),
    );
    for theme in themes {
        out += "\n";
        out += &wrap(theme.about(), &format!("# {}: ", theme.name()), "#   ");
        for param in theme.params() {
            out += &wrap(param.about, &format!("#   {:<16}", param.key), &format!("#   {:16}", ""));
        }
        for variant in theme.variants() {
            out += &format!(
                "\n[themes.{}]\nfont_family = \"{}\"\nfont_size = {}\nfont_weight = {}\nline_height = {}\n",
                variant.key(),
                variant.font_family,
                variant.font_size,
                variant.font_weight,
                variant.line_height,
            );
            for (param, value) in theme.params().iter().zip(&variant.params) {
                out += &format!("{} = {value}\n", param.key);
            }
        }
    }
    out
}

/// `text` as comment lines of at most 80 characters, the first starting
/// with `first`, the others with `rest`.
fn wrap(text: &str, first: &str, rest: &str) -> String {
    let mut out = String::new();
    let mut line = first.to_string();
    let mut empty = true;
    for word in text.split_whitespace() {
        if !empty && line.chars().count() + 1 + word.chars().count() > 80 {
            out += &line;
            out += "\n";
            line = rest.to_string();
            empty = true;
        }
        if !empty {
            line.push(' ');
        }
        line += word;
        empty = false;
    }
    out += &line;
    out += "\n";
    out
}

/// A parse error in words that say how to fix it. The most common mistake
/// is a per-theme setting outside its section, which toml reports as an
/// unknown field followed by the whole list of valid ones.
fn explain(message: &str) -> String {
    let Some(name) = message.strip_prefix("unknown field `").and_then(|rest| rest.split('`').next()) else {
        return message.trim().to_string();
    };
    let sections: Vec<String> = theme::builtin()
        .iter()
        .filter(|v| FONT_SETTINGS.contains(&name) || v.theme.params().iter().any(|p| p.key == name))
        .map(|v| format!("[themes.{}]", v.key()))
        .collect();
    if !sections.is_empty() {
        let sections = if sections.len() > 2 { format!("{} or ...", sections[..2].join(", ")) } else { sections.join(" or ") };
        return format!("`{name}` is set per theme: put it under {sections}");
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
        let ids: Vec<&str> = theme::all().iter().map(|t| t.id()).collect();
        if let Some(name) = self.theme.as_deref().filter(|name| !ids.contains(name)) {
            return Err(format!("unknown theme \"{name}\" (there are: {})", ids.join(", ")));
        }
        let variants = theme::builtin();
        for (section, o) in &self.themes {
            let Some(variant) = variants.iter().find(|v| v.key() == key(section)) else {
                return Err(format!("unknown theme section [themes.{section}]"));
            };
            for (name, &value) in &o.params {
                let Some(param) = variant.theme.params().iter().find(|p| p.key == name) else {
                    return Err(format!("unknown setting `{name}` in [themes.{section}]"));
                };
                if !(param.min..=param.max).contains(&value) {
                    return Err(format!("{name} {value} is out of range ({} to {})", param.min, param.max));
                }
            }
        }
        let sizes = std::iter::once(self.font_size)
            .chain(self.themes.values().map(|o| o.font_size))
            .flatten();
        if let Some(size) = sizes.into_iter().find(|s| !(4.0..=96.0).contains(s)) {
            return Err(format!("font_size {size} is out of range (4 to 96)"));
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

    /// Every theme's variants with this config's overrides applied.
    pub fn themes(&self) -> Vec<Variant> {
        let mut variants = theme::builtin();
        for variant in &mut variants {
            apply_fonts(variant, &self.font_family, self.font_size, self.font_weight);
            let Some(o) = self.themes.iter().find(|(k, _)| key(k) == variant.key()).map(|(_, o)| o) else {
                continue;
            };
            apply_fonts(variant, &o.font_family, o.font_size, o.font_weight);
            if let Some(v) = o.line_height {
                variant.line_height = v;
            }
            for (i, param) in variant.theme.params().iter().enumerate() {
                if let (Some(&value), Some(slot)) = (o.params.get(param.key), variant.params.get_mut(i)) {
                    *slot = value;
                }
            }
        }
        variants
    }

    /// Index of the variant to show: of the chosen theme (`picked` in the
    /// picker this session, else the settings, else the first), the one for
    /// the appearance, given whether the system is in dark mode and any
    /// session override from Cmd+Shift+L.
    pub fn theme_index(&self, variants: &[Variant], system_dark: bool, flip: Option<bool>, picked: Option<&str>) -> usize {
        let dark = flip.unwrap_or(match self.appearance {
            Appearance::System => system_dark,
            Appearance::Light => false,
            Appearance::Dark => true,
        });
        let id = picked.or(self.theme.as_deref()).unwrap_or_else(|| theme::all()[0].id());
        let of_theme = |v: &Variant| v.theme.id() == id;
        variants
            .iter()
            .position(|v| of_theme(v) && v.dark == dark)
            .or_else(|| variants.iter().position(of_theme))
            .unwrap_or(0)
    }
}

fn apply_fonts(theme: &mut Variant, family: &Option<String>, size: Option<f32>, weight: Option<u16>) {
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

/// Makes `id` the theme in the settings file: replaces its `theme = ...`
/// line, or adds one before the first setting, keeping everything else as
/// the user wrote it. A missing file is created from the template.
pub fn save_theme(path: &Path, id: &str) -> std::io::Result<()> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => template(),
        Err(err) => return Err(err),
    };
    let line = format!("theme = \"{id}\"");
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    // Only top-level lines count: before the first [section].
    let top = lines.iter().position(|l| l.trim_start().starts_with('[')).unwrap_or(lines.len());
    let is_theme = |l: &String| l.split('=').next().is_some_and(|k| k.trim() == "theme") && l.contains('=');
    match lines[..top].iter().position(is_theme) {
        Some(i) => lines[i] = line,
        None => {
            let first = lines[..top].iter().position(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'));
            lines.insert(first.unwrap_or(top), line);
        }
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, lines.join("\n") + "\n")
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
        let (ours, builtin) = (config.themes(), theme::builtin());
        assert_eq!(ours.len(), builtin.len());
        for (a, b) in ours.iter().zip(&builtin) {
            assert_eq!(
                (&a.font_family, a.font_size, a.font_weight, a.line_height),
                (&b.font_family, b.font_size, b.font_weight, b.line_height),
                "{}",
                a.name
            );
            assert_eq!(a.params, b.params, "{}", a.name);
        }
        assert_eq!(config.appearance, Appearance::System);
    }

    #[test]
    fn paper_strengths_are_settable() {
        let config = Config::parse("[themes.tsushima-light]\ntexture = 0.5\nstains = 0\nsplatter = 2").unwrap();
        let light = config.themes().into_iter().find(|t| !t.dark).unwrap();
        assert_eq!((light.param("texture"), light.param("stains"), light.param("splatter")), (0.5, 0.0, 2.0));
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
        assert_eq!((light.param("bleed_variation"), light.param("bleed_time")), (0.6, 4.0));
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
        assert_eq!(light.param("nib"), 0.5);
        // Line height scales with the size: 24 at 17pt becomes 28 at 20pt.
        assert_eq!(light.line_height, 28.0);
        // The global size reaches the dark variant; the light-only font does not.
        let dark = themes.iter().find(|t| t.dark).unwrap();
        assert_eq!(dark.font_size, 20.0);
        assert_eq!(dark.font_family, "Courier New");
    }

    #[test]
    fn appearance_picks_the_variant() {
        let themes = theme::builtin();
        let pick = |text: &str, system_dark, flip| {
            let config = Config::parse(text).unwrap();
            themes[config.theme_index(&themes, system_dark, flip, None)].dark
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
        assert!(Config::parse("theme = \"nope\"").unwrap_err().contains("tsushima"));
        assert!(Config::parse("theme = \"tsushima\"").is_ok());
    }

    #[test]
    fn saving_a_theme_keeps_the_rest_of_the_file() {
        let dir = std::env::temp_dir().join(format!("oneprompt-save-theme-{}", std::process::id()));
        let path = dir.join("config.toml");
        // No file: the template, with the theme set.
        save_theme(&path, "chalkboard").unwrap();
        assert_eq!(Config::load(&path).unwrap().theme.as_deref(), Some("chalkboard"));
        // An existing line is replaced; a file without one gets one on top.
        save_theme(&path, "tsushima").unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text.matches("theme = ").count(), 1);
        std::fs::write(&path, "# mine\nappearance = \"dark\"\n[themes.tsushima-dark]\nnib = 2\n").unwrap();
        save_theme(&path, "chalkboard").unwrap();
        let config = Config::load(&path).unwrap();
        assert_eq!((config.theme.as_deref(), config.appearance), (Some("chalkboard"), Appearance::Dark));
        assert!(std::fs::read_to_string(&path).unwrap().starts_with("# mine\ntheme = \"chalkboard\"\n"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn theme_keys_ignore_case_and_spaces() {
        assert_eq!(key("Tsushima Dark"), "tsushima-dark");
        assert_eq!(key(" tsushima_dark "), "tsushima-dark");
    }
}
