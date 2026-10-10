//! Settings file: `~/.config/oneprompt/config.toml` (or `$XDG_CONFIG_HOME`).
//!
//! Cmd+, opens the settings panel (settings.rs), which writes its changes
//! here with `apply_edits`, keeping the user's comments and layout; Cmd+O in
//! the panel opens the file itself in the default editor (creating it from
//! `template()` first). A watcher thread reloads it whenever it is saved, so
//! every setting applies live, however it was changed. Everything is
//! optional; a missing file means defaults.

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

/// Weather over the window.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum WeatherSetting {
    #[default]
    None,
    Rain,
    Snow,
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
    /// Fresh ink looks wet for `drying_time` seconds (default 4) and dries.
    /// Default on.
    pub ink_drying: Option<bool>,
    pub drying_time: Option<f32>,
    /// How far ink an hour old has faded, 0..1 (default 0.3; 0: never).
    pub ink_aging: Option<f32>,
    /// The cursor flicks a trail of ink when it jumps, and beats gently
    /// while it is still. Default on.
    pub cursor_trail: Option<bool>,
    pub cursor_pulse: Option<bool>,
    /// Sheets age with use: a tab open `paper_aging_hours` (default 8) has a
    /// worn sheet; a new tab starts fresh. Default on.
    pub paper_aging: Option<bool>,
    pub paper_aging_hours: Option<f32>,
    /// Pasted text lands at once, as a splash of ink. Default on.
    pub paste_splash: Option<bool>,
    /// Rain or snow falling over the window, `weather_amount` heavy (0..1,
    /// default 0.5). Default none.
    pub weather: WeatherSetting,
    pub weather_amount: Option<f32>,
    /// Closing a tab or the window, and clearing the screen (Cmd+K), the
    /// theme's way. Default on.
    pub close_effect: Option<bool>,
    pub clear_effect: Option<bool>,
    /// After idle_after seconds (default 60) with no key or click, the theme's
    /// idle scene plays: Tsushima's falling leaves. Default on.
    pub idle_scene: Option<bool>,
    pub idle_after: Option<f32>,
    /// Have the shell mark its prompts (marks.rs), for what follows. Default
    /// on; applies to shells started after it changes.
    pub shell_integration: Option<bool>,
    /// Mark commands that failed beside their prompt. Default on.
    pub mark_failures: Option<bool>,
    /// Stamp commands that ran at least `stamp_after` seconds (default 10)
    /// when they finish. Default on.
    pub stamp_long_commands: Option<bool>,
    pub stamp_after: Option<f32>,
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

# Fresh ink is wet: darker and glossy in Tsushima, brighter and dustier chalk
# on the Chalkboard, drying over drying_time seconds. Ink that has been on
# screen a long while fades a little, as old ink does: ink_aging is how far
# (0 never; reached after an hour).
ink_drying = true
drying_time = 4
ink_aging = 0.3

# The cursor is a brush (a stick of chalk on the Chalkboard): it flicks a
# trail behind it when it jumps, and while it is still it beats gently, like
# a heart, so it is easy to find.
cursor_trail = true
cursor_pulse = true

# Pasted text lands all at once, as a splash of ink that settles into the
# letters (a puff of chalk on the Chalkboard), instead of being written in.
paste_splash = true

# The sheet ages as you use it: over paper_aging_hours of a tab being open,
# Tsushima's washi yellows and picks up creases from being folded; the
# Chalkboard collects smudges and ghosts of old lessons. A new tab starts on
# a fresh sheet.
paper_aging = true
paper_aging_hours = 8

# Weather over the paper: "none", "rain" (sumi-e strokes of rain, drops
# blotting where they land; chalk lines on the Chalkboard) or "snow". It keeps
# the window drawing while it has focus, so it costs a little battery.
weather = "none"
weather_amount = 0.5         # 0 a few drops .. 1 a downpour

# Closing a tab or the window burns its sheet away in Tsushima and wipes it off
# the Chalkboard. Clearing the screen (Cmd+K) cuts it in two with a sword in
# Tsushima, the halves sliding apart and falling; the Chalkboard gets one
# sweep of the eraser.
close_effect = true
clear_effect = true

# Left alone for idle_after seconds, the window gets the theme's idle scene:
# maple leaves drift down onto the paper in Tsushima, chalk dust settles on
# the Chalkboard. The next key or click sweeps it away.
idle_scene = true
idle_after = 60

# The shell marks its prompts so the terminal knows where each command starts
# and how it ended (zsh: automatic; fish 4: built in; bash: see the README).
# Takes effect in shells started after it changes.
shell_integration = true
# A command that fails gets the theme's mark beside its prompt: a stroke of
# blood-red ink in Tsushima, a chalk cross on the Chalkboard. Ctrl+C is not a
# failure.
mark_failures = true
# A command that ran a while is stamped when it finishes, so it catches the eye:
# a seal pressed on in Tsushima, a circled tick on the Chalkboard (a cross if it
# failed).
stamp_long_commands = true
stamp_after = 10             # seconds a command must run to be stamped

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
        if let Some(hours) = self.paper_aging_hours
            && !(0.1..=1000.0).contains(&hours)
        {
            return Err(format!("paper_aging_hours {hours} is out of range (0.1 to 1000)"));
        }
        if let Some(amount) = self.weather_amount
            && !(0.0..=1.0).contains(&amount)
        {
            return Err(format!("weather_amount {amount} is out of range (0 to 1)"));
        }
        if let Some(time) = self.drying_time
            && !(0.5..=60.0).contains(&time)
        {
            return Err(format!("drying_time {time} is out of range (0.5 to 60)"));
        }
        if let Some(aging) = self.ink_aging
            && !(0.0..=1.0).contains(&aging)
        {
            return Err(format!("ink_aging {aging} is out of range (0 to 1)"));
        }
        if let Some(after) = self.idle_after
            && !(5.0..=86_400.0).contains(&after)
        {
            return Err(format!("idle_after {after} is out of range (5 to 86400)"));
        }
        if let Some(after) = self.stamp_after
            && !(1.0..=86_400.0).contains(&after)
        {
            return Err(format!("stamp_after {after} is out of range (1 to 86400)"));
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

/// A value the settings panel writes.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Str(String),
    Num(f64),
    Bool(bool),
}

/// One setting changed in the settings panel: top-level (`section` None)
/// or in a variant's `[themes.<section>]`.
#[derive(Clone, Debug, PartialEq)]
pub struct Edit {
    pub section: Option<String>,
    pub key: String,
    pub value: Value,
}

/// The settings file's text, or the template when there is none yet.
pub fn read_or_template(path: &Path) -> std::io::Result<String> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(text),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(template()),
        Err(err) => Err(err),
    }
}

/// `text` with `edits` made, keeping everything else as the user wrote it:
/// comments (also the one after a changed value), spacing and order. A
/// setting not in the file yet is added to its table; a variant's section
/// is found however it is spelled (`[themes."Tsushima Light"]`) or added.
pub fn apply_edits(text: &str, edits: &[Edit]) -> Result<String, String> {
    use toml_edit::{DocumentMut, Item, Table};
    let mut doc: DocumentMut = text.parse().map_err(|e: toml_edit::TomlError| e.message().trim().to_string())?;
    for edit in edits {
        let table: &mut dyn toml_edit::TableLike = match &edit.section {
            None => doc.as_table_mut(),
            Some(section) => {
                if !doc.contains_key("themes") {
                    let mut themes = Table::new();
                    themes.set_implicit(true);
                    doc.insert("themes", Item::Table(themes));
                }
                let themes = doc["themes"].as_table_like_mut().ok_or("`themes` in the settings file is not a table")?;
                let name = themes
                    .iter()
                    .map(|(name, _)| name.to_string())
                    .find(|name| key(name) == key(section))
                    .unwrap_or_else(|| key(section));
                if !themes.contains_key(&name) {
                    themes.insert(&name, Item::Table(Table::new()));
                }
                themes
                    .get_mut(&name)
                    .and_then(Item::as_table_like_mut)
                    .ok_or_else(|| format!("[themes.{name}] in the settings file is not a table"))?
            }
        };
        let mut value = match &edit.value {
            Value::Str(s) => toml_edit::Value::from(s.as_str()),
            // Whole numbers are written as integers, as in the template.
            Value::Num(n) if n.fract() == 0.0 && n.abs() < 1e9 => toml_edit::Value::from(*n as i64),
            Value::Num(n) => toml_edit::Value::from(*n),
            Value::Bool(b) => toml_edit::Value::from(*b),
        };
        match table.get_mut(&edit.key).and_then(Item::as_value_mut) {
            Some(old) => {
                *value.decor_mut() = old.decor().clone();
                *old = value;
            }
            None => {
                table.insert(&edit.key, Item::Value(value));
            }
        }
    }
    Ok(doc.to_string())
}

/// Writes the settings file, creating its folder if needed.
pub fn write(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, text)
}

/// Creates the settings file from the template if needed, then opens it in
/// the system's default text editor.
pub fn open_in_editor(path: &Path) -> std::io::Result<()> {
    if !path.exists() {
        write(path, &template())?;
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
        assert!(Config::parse("stamp_after = 0").unwrap_err().contains("out of range"));
        assert!(Config::parse("ink_aging = 2").unwrap_err().contains("out of range"));
        assert!(Config::parse("idle_after = 1").unwrap_err().contains("out of range"));
        assert_eq!(Config::parse("weather = \"snow\"").unwrap().weather, WeatherSetting::Snow);
        assert!(Config::parse("weather = \"hail\"").is_err());
        assert!(Config::parse("drying_time = 0.1").unwrap_err().contains("out of range"));
        assert!(Config::parse("stamp_after = 30\nstamp_long_commands = false").is_ok());
        assert!(Config::parse("theme = \"nope\"").unwrap_err().contains("tsushima"));
        assert!(Config::parse("theme = \"tsushima\"").is_ok());
    }

    #[test]
    fn edits_keep_the_rest_of_the_file() {
        let edit = |section: Option<&str>, key: &str, value| Edit { section: section.map(str::to_string), key: key.into(), value };
        let text = apply_edits(
            &template(),
            &[
                edit(None, "animation_speed", Value::Num(2.5)),
                edit(None, "animation", Value::Bool(false)),
                edit(Some("Tsushima Light"), "nib", Value::Num(3.0)),
                edit(Some("chalkboard-slate"), "font_family", Value::Str("Menlo".into())),
            ],
        )
        .unwrap();
        // The comment after the changed value stays, and so does the rest.
        assert!(text.contains("animation_speed = 2.5        # 2 = twice as fast"), "{text}");
        assert_eq!(text.lines().count(), template().lines().count());
        let config = Config::parse(&text).unwrap();
        assert_eq!(config.animation(), (false, 2.5, 500.0));
        let themes = config.themes();
        let light = themes.iter().find(|v| v.key() == "tsushima-light").unwrap();
        assert_eq!(light.param("nib"), 3.0);
        assert!(text.contains("\nnib = 3\n"));

        // Settings and sections not in the file yet are added, wherever the
        // section is and however it is spelled.
        let text = apply_edits(
            "# mine\n[themes.\"Tsushima Dark\"]\nbleed = 1\n",
            &[edit(Some("tsushima-dark"), "nib", Value::Num(0.5)), edit(Some("chalkboard-stone"), "grit", Value::Num(0.25)), edit(None, "theme", Value::Str("chalkboard".into()))],
        )
        .unwrap();
        let config = Config::parse(&text).unwrap();
        assert_eq!(config.theme.as_deref(), Some("chalkboard"));
        let themes = config.themes();
        let get = |k: &str, p: &str| themes.iter().find(|v| v.key() == k).unwrap().param(p);
        assert_eq!((get("tsushima-dark", "bleed"), get("tsushima-dark", "nib"), get("chalkboard-stone", "grit")), (1.0, 0.5, 0.25));
        assert!(text.contains("# mine\n[themes.\"Tsushima Dark\"]\nbleed = 1\nnib = 0.5\n"), "{text}");
        assert!(apply_edits("x = = 1", &[]).is_err());
    }

    #[test]
    fn theme_keys_ignore_case_and_spaces() {
        assert_eq!(key("Tsushima Dark"), "tsushima-dark");
        assert_eq!(key(" tsushima_dark "), "tsushima-dark");
    }
}
