//! The themes, each in its own folder, built only on `crate::theme` (see
//! the API there). Adding one: a folder with a `mod.rs` implementing
//! `Theme`, its shaders next to it, and a line in `all()`.

mod chalkboard;
mod tsushima;

use crate::theme::Theme;

/// Every theme, in picker order. The first is the default.
pub fn all() -> Vec<&'static dyn Theme> {
    vec![&tsushima::TSUSHIMA, &chalkboard::CHALKBOARD]
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    /// A theme reaches the app only through the theme API, and never into
    /// another theme: each one can be read, changed or removed on its own.
    #[test]
    fn themes_only_use_the_theme_api() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/themes");
        for theme in std::fs::read_dir(&root).unwrap().flatten().filter(|e| e.path().is_dir()) {
            let name = theme.file_name().to_string_lossy().to_string();
            for file in std::fs::read_dir(theme.path()).unwrap().flatten() {
                let path = file.path();
                if path.extension().is_none_or(|e| e != "rs") {
                    continue;
                }
                let source = std::fs::read_to_string(&path).unwrap();
                for (i, line) in source.lines().enumerate() {
                    let mut rest = line;
                    while let Some(at) = rest.find("crate::") {
                        rest = &rest[at + "crate::".len()..];
                        assert!(
                            rest.starts_with("theme") && !rest.starts_with("themes"),
                            "{}:{}: theme `{name}` uses `crate::{}`; themes may only use `crate::theme`",
                            path.display(),
                            i + 1,
                            rest.split(|c: char| !c.is_alphanumeric() && c != '_').next().unwrap_or("")
                        );
                    }
                    assert!(!line.contains("super::super"), "{}:{}: reaches outside its theme", path.display(), i + 1);
                }
            }
        }
    }
}
