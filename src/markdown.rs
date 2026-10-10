//! `oneprompt md FILE`: Markdown shown in the terminal, styled. Headings,
//! emphasis, code, lists, task lists, quotes, tables, rules and links come
//! out as text with ANSI styles, using only the 16 colours, so every theme
//! draws them in its own inks; local images are shown inline (images.rs),
//! links can be Cmd+clicked (OSC 8). Text is wrapped to the terminal's width.

use std::fmt::Write as _;
use std::path::Path;

use pulldown_cmark::{Alignment, CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

/// Styles, as SGR parameters. Colours are the theme's 16 (its inks).
const BOLD: &str = "1";
const DIM: &str = "2";
const ITALIC: &str = "3";
const UNDERLINE: &str = "4";
const STRIKE: &str = "9";
const RED: &str = "31";
const BLUE: &str = "34";
const CYAN: &str = "36";

/// Renders `source` (Markdown) for a terminal `width` columns wide; images
/// are looked up from `base`.
pub fn render(source: &str, width: usize, base: &Path) -> String {
    let options = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let mut w = Writer::new(width.max(20));
    let mut lists: Vec<Option<u64>> = Vec::new();
    let mut code: Option<String> = None;
    let mut table: Option<Table> = None;
    let mut image: Option<(String, String)> = None;
    let mut link_depth = 0;
    for event in Parser::new_ext(source, options) {
        if let Some(t) = table.as_mut() {
            t.take(&event);
            if t.done {
                let lines = t.lines(w.width);
                table = None;
                w.blank();
                for line in lines {
                    w.line_raw(&line);
                }
                w.end_block();
            }
            continue;
        }
        if let Some((_, alt)) = image.as_mut()
            && let Event::Text(text) = &event
        {
            alt.push_str(text);
            continue;
        }
        match event {
            Event::Start(tag) => match tag {
                Tag::Paragraph => w.block(),
                Tag::Heading { level, .. } => {
                    w.blank();
                    let style = match level {
                        HeadingLevel::H1 => vec![BOLD, RED],
                        HeadingLevel::H2 => vec![BOLD, BLUE],
                        _ => vec![BOLD],
                    };
                    w.push_style(&style);
                    w.heading = Some(level);
                }
                Tag::BlockQuote(_) => {
                    w.block();
                    w.indent.push(("\x1b[2m\u{2503}\x1b[0m ".into(), 2));
                    w.push_style(&[ITALIC]);
                }
                Tag::CodeBlock(kind) => {
                    w.blank();
                    if let CodeBlockKind::Fenced(lang) = kind
                        && !lang.is_empty()
                    {
                        w.line_raw(&format!("\x1b[{DIM}m  {lang}\x1b[0m"));
                    }
                    code = Some(String::new());
                }
                Tag::List(start) => {
                    if lists.is_empty() {
                        w.block();
                    }
                    lists.push(start);
                }
                Tag::Item => {
                    w.newline();
                    let marker = match lists.last_mut() {
                        Some(Some(n)) => {
                            *n += 1;
                            format!("{}.", *n - 1)
                        }
                        _ => "\u{2022}".into(),
                    };
                    // Nested lists are indented by their parents' hang.
                    let hang = marker.chars().count() + 1;
                    w.word_raw(&format!("\x1b[{RED}m{marker}\x1b[0m "), hang);
                    w.indent.push((" ".repeat(hang), hang));
                    w.item_open = true;
                }
                Tag::Emphasis => w.push_style(&[ITALIC]),
                Tag::Strong => w.push_style(&[BOLD]),
                Tag::Strikethrough => w.push_style(&[STRIKE]),
                Tag::Link { dest_url, .. } => {
                    link_depth += 1;
                    w.raw(&format!("\x1b]8;;{dest_url}\x1b\\"));
                    w.push_style(&[UNDERLINE, BLUE]);
                }
                Tag::Image { dest_url, .. } => image = Some((dest_url.to_string(), String::new())),
                Tag::Table(align) => table = Some(Table::new(align)),
                _ => {}
            },
            Event::End(end) => match end {
                TagEnd::Paragraph => w.end_block(),
                TagEnd::Heading(level) => {
                    let len = w.col.saturating_sub(w.indent_width());
                    w.pop_style();
                    w.heading = None;
                    if matches!(level, HeadingLevel::H1 | HeadingLevel::H2) {
                        let rule = if level == HeadingLevel::H1 { '\u{2501}' } else { '\u{2500}' };
                        w.line_raw(&format!("\x1b[{DIM}m{}\x1b[0m", rule.to_string().repeat(len.max(3))));
                    }
                    w.end_block();
                }
                TagEnd::BlockQuote(_) => {
                    w.pop_style();
                    w.indent.pop();
                    w.end_block();
                }
                TagEnd::CodeBlock => {
                    if let Some(text) = code.take() {
                        for line in text.trim_end_matches('\n').lines() {
                            w.line_raw(&format!("\x1b[{DIM}m  \u{2502}\x1b[0m \x1b[{CYAN}m{line}\x1b[0m"));
                        }
                    }
                    w.end_block();
                }
                TagEnd::List(_) => {
                    lists.pop();
                    if lists.is_empty() {
                        w.end_block();
                    }
                }
                TagEnd::Item => {
                    w.indent.pop();
                    w.item_open = false;
                }
                TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough => w.pop_style(),
                TagEnd::Link => {
                    w.pop_style();
                    link_depth -= 1;
                    w.raw("\x1b]8;;\x1b\\");
                }
                TagEnd::Image => {
                    if let Some((dest, alt)) = image.take() {
                        w.picture(&dest, &alt, base);
                    }
                }
                _ => {}
            },
            Event::Text(text) => match code.as_mut() {
                Some(code) => code.push_str(&text),
                None => w.text(&text),
            },
            Event::Code(text) => w.word(&format!("\x1b[{CYAN}m{text}\x1b[0m"), text.chars().count()),
            Event::SoftBreak => w.space = true,
            Event::HardBreak => w.newline(),
            Event::Rule => {
                w.blank();
                let width = w.width.saturating_sub(w.indent_width());
                w.line_raw(&format!("\x1b[{DIM}m{}\x1b[0m", "\u{2500}".repeat(width)));
                w.end_block();
            }
            Event::TaskListMarker(done) => {
                let mark = if done { format!("\x1b[{RED}m[x]\x1b[0m") } else { "[ ]".into() };
                w.word(&mark, 3);
                w.space = true;
            }
            Event::Html(html) | Event::InlineHtml(html) => w.word(&format!("\x1b[{DIM}m{}\x1b[0m", html.trim()), html.trim().chars().count()),
            _ => {}
        }
    }
    let _ = link_depth;
    w.finish()
}

/// Wraps styled words into lines.
struct Writer {
    width: usize,
    out: String,
    /// The line being written, its visible width, whether a space is due
    /// before the next word, and whether the last line is blank.
    col: usize,
    space: bool,
    blank_before: bool,
    started: bool,
    /// Prefixes every line inside (quote bars, list hangs): text and width.
    indent: Vec<(String, usize)>,
    /// The SGR styles open, innermost last.
    styles: Vec<Vec<&'static str>>,
    heading: Option<HeadingLevel>,
    item_open: bool,
}

impl Writer {
    fn new(width: usize) -> Self {
        Self { width, out: String::new(), col: 0, space: false, blank_before: true, started: false, indent: Vec::new(), styles: Vec::new(), heading: None, item_open: false }
    }

    fn indent_width(&self) -> usize {
        self.indent.iter().map(|(_, w)| w).sum()
    }

    fn sgr(&self) -> String {
        let all: Vec<&str> = self.styles.iter().flatten().copied().collect();
        if all.is_empty() { String::new() } else { format!("\x1b[{}m", all.join(";")) }
    }

    fn push_style(&mut self, style: &[&'static str]) {
        self.styles.push(style.to_vec());
    }

    fn pop_style(&mut self) {
        self.styles.pop();
    }

    /// Starts a line: its indent.
    fn begin_line(&mut self) {
        if self.col == 0 {
            let prefix: String = self.indent.iter().map(|(t, _)| t.as_str()).collect();
            self.out.push_str(&prefix);
            self.col = self.indent_width();
        }
    }

    fn raw(&mut self, text: &str) {
        self.out.push_str(text);
    }

    fn newline(&mut self) {
        if self.col > 0 {
            self.out.push('\n');
            self.blank_before = false;
        }
        self.col = 0;
        self.space = false;
    }

    /// A paragraph-like block starts: a blank line before it, unless at the
    /// top or in a list item.
    fn block(&mut self) {
        if !self.item_open {
            self.blank();
        }
    }

    fn blank(&mut self) {
        self.newline();
        if self.started && !self.blank_before {
            self.out.push('\n');
            self.blank_before = true;
        }
    }

    fn end_block(&mut self) {
        self.newline();
        self.started = true;
    }

    /// A whole line, already styled (code, rules, tables), not wrapped.
    fn line_raw(&mut self, line: &str) {
        self.newline();
        self.begin_line();
        self.out.push_str(line);
        self.out.push('\n');
        self.col = 0;
        self.blank_before = false;
        self.started = true;
    }

    /// A word, already styled, `len` columns wide, on this line or the next.
    fn word(&mut self, styled: &str, len: usize) {
        let gap = if self.space && self.col > self.indent_width() { 1 } else { 0 };
        if self.col > self.indent_width() && self.col + gap + len > self.width {
            self.newline();
        }
        self.begin_line();
        if self.space && self.col > self.indent_width() {
            self.out.push(' ');
            self.col += 1;
        }
        let sgr = self.sgr();
        self.out.push_str(&sgr);
        self.out.push_str(styled);
        if !sgr.is_empty() {
            self.out.push_str("\x1b[0m");
        }
        self.col += len;
        self.space = false;
        self.started = true;
    }

    /// Something placed as it is at the start of a line (a list marker).
    fn word_raw(&mut self, text: &str, len: usize) {
        self.begin_line();
        self.out.push_str(text);
        self.col += len;
        self.space = false;
    }

    /// Running text, broken into words.
    fn text(&mut self, text: &str) {
        if text.starts_with(char::is_whitespace) {
            self.space = true;
        }
        let mut words = text.split_whitespace().peekable();
        while let Some(word) = words.next() {
            self.word(word, word.chars().count());
            if words.peek().is_some() {
                self.space = true;
            }
        }
        if text.ends_with(char::is_whitespace) {
            self.space = true;
        }
    }

    /// An image: shown inline if it is a file here, else its alt text,
    /// linked.
    fn picture(&mut self, dest: &str, alt: &str, base: &Path) {
        let path = base.join(dest);
        let local = !dest.contains("://") && path.is_file();
        if let Some(bytes) = local.then(|| std::fs::read(&path).ok()).flatten() {
            self.newline();
            let name = path.file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned());
            // A picture in a document sits in it: half the width at most.
            let width = (self.width / 2).clamp(10, 40);
            self.out.push_str(&crate::images::sequence(&bytes, &name, Some(width)));
            self.col = 0;
            self.started = true;
            self.blank_before = false;
        } else {
            let label = if alt.is_empty() { "image".to_string() } else { alt.to_string() };
            let text = format!("\x1b]8;;{dest}\x1b\\\x1b[{DIM}m[{label}]\x1b[0m\x1b]8;;\x1b\\");
            self.word(&text, label.chars().count() + 2);
        }
    }

    fn finish(mut self) -> String {
        self.newline();
        while self.out.ends_with('\n') {
            self.out.pop();
        }
        self.out
    }
}

/// A table being read, then drawn with box-drawing lines.
struct Table {
    align: Vec<Alignment>,
    rows: Vec<Vec<String>>,
    cell: Option<String>,
    head_rows: usize,
    in_head: bool,
    done: bool,
}

impl Table {
    fn new(align: Vec<Alignment>) -> Self {
        Self { align, rows: Vec::new(), cell: None, head_rows: 0, in_head: false, done: false }
    }

    /// Takes `event`, which belongs to the table.
    fn take(&mut self, event: &Event) {
        match event {
            Event::Start(Tag::TableHead) => {
                self.in_head = true;
                self.rows.push(Vec::new());
            }
            Event::End(TagEnd::TableHead) => {
                self.in_head = false;
                self.head_rows = self.rows.len();
            }
            Event::Start(Tag::TableRow) => self.rows.push(Vec::new()),
            Event::Start(Tag::TableCell) => self.cell = Some(String::new()),
            Event::End(TagEnd::TableCell) => {
                let cell = self.cell.take().unwrap_or_default();
                if let Some(row) = self.rows.last_mut() {
                    row.push(cell.trim().to_string());
                }
            }
            Event::Text(t) | Event::Code(t) => {
                if let Some(c) = self.cell.as_mut() {
                    c.push_str(t);
                }
            }
            Event::End(TagEnd::Table) => self.done = true,
            _ => {}
        }
    }

    /// The table's lines, its columns shrunk to fit `width` if need be.
    fn lines(&self, width: usize) -> Vec<String> {
        let columns = self.rows.iter().map(Vec::len).max().unwrap_or(0);
        if columns == 0 {
            return Vec::new();
        }
        let mut widths: Vec<usize> = (0..columns)
            .map(|c| self.rows.iter().filter_map(|r| r.get(c)).map(|s| s.chars().count()).max().unwrap_or(0).max(1))
            .collect();
        // Shrink the widest columns until it fits: borders take 3 a column + 1.
        while widths.iter().sum::<usize>() + columns * 3 + 1 > width && widths.iter().any(|&w| w > 4) {
            let i = (0..columns).max_by_key(|&i| widths[i]).unwrap_or(0);
            widths[i] -= 1;
        }
        let rule = |l: &str, m: &str, r: &str| {
            let mut s = format!("\x1b[{DIM}m{l}");
            for (i, w) in widths.iter().enumerate() {
                s.push_str(&"\u{2500}".repeat(w + 2));
                s.push_str(if i + 1 < columns { m } else { r });
            }
            s.push_str("\x1b[0m");
            s
        };
        let mut out = vec![rule("\u{250c}", "\u{252c}", "\u{2510}")];
        for (r, row) in self.rows.iter().enumerate() {
            let mut line = format!("\x1b[{DIM}m\u{2502}\x1b[0m");
            for (c, w) in widths.iter().enumerate() {
                let text: String = row.get(c).map_or("", String::as_str).chars().take(*w).collect();
                let pad = w - text.chars().count();
                let (left, right) = match self.align.get(c) {
                    Some(Alignment::Right) => (pad, 0),
                    Some(Alignment::Center) => (pad / 2, pad - pad / 2),
                    _ => (0, pad),
                };
                let styled = if r < self.head_rows { format!("\x1b[{BOLD}m{text}\x1b[0m") } else { text };
                let _ = write!(line, " {}{styled}{} \x1b[{DIM}m\u{2502}\x1b[0m", " ".repeat(left), " ".repeat(right));
            }
            out.push(line);
            if r + 1 == self.head_rows {
                out.push(rule("\u{251c}", "\u{253c}", "\u{2524}"));
            }
        }
        out.push(rule("\u{2514}", "\u{2534}", "\u{2518}"));
        out
    }
}

/// The terminal's width in columns, if standard output is one.
pub fn terminal_width() -> Option<usize> {
    let mut size: libc::winsize = unsafe { std::mem::zeroed() };
    // SAFETY: TIOCGWINSZ fills in a winsize for a terminal file descriptor.
    let ok = unsafe { libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut size) } == 0;
    (ok && size.ws_col > 0).then_some(size.ws_col as usize)
}

/// Removes escape sequences, for plain output (not a terminal, or NO_COLOR).
pub fn plain(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\x1b' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('[') => {
                for c in chars.by_ref() {
                    if c.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            Some(']') => {
                while let Some(c) = chars.next() {
                    if c == '\x07' || (c == '\x1b' && chars.next_if_eq(&'\\').is_some()) {
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn show(md: &str, width: usize) -> String {
        plain(&render(md, width, Path::new(".")))
    }

    #[test]
    fn markdown_comes_out_styled_and_wrapped() {
        let text = render("# Title\n\nSome *soft* and **bold** with `code`.\n", 40, Path::new("."));
        assert!(text.contains("\x1b[1;31mTitle"), "{text:?}");
        assert!(text.contains("\x1b[3msoft") && text.contains("\x1b[1mbold") && text.contains("\x1b[36mcode"));
        assert_eq!(show("# Title\n\nSome text.\n", 40), "Title\n\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\n\nSome text.");
        // Wrapped to the width, words whole.
        let wrapped = show("one two three four five six seven eight nine ten", 20);
        assert!(wrapped.lines().all(|l| l.chars().count() <= 20), "{wrapped}");
        assert_eq!(wrapped.split_whitespace().count(), 10);
    }

    #[test]
    fn lists_quotes_code_and_tables() {
        let text = show("- one\n- two\n\n1. first\n2. second\n", 40);
        assert!(text.contains("\u{2022} one") && text.contains("\u{2022} two") && text.contains("1. first") && text.contains("2. second"), "{text}");
        let quote = show("> said\n", 40);
        assert!(quote.contains("\u{2503} said"), "{quote}");
        let code = show("```rust\nfn main() {}\n```\n", 40);
        assert!(code.contains("rust") && code.contains("\u{2502} fn main() {}"), "{code}");
        let table = show("| a | bb |\n|---|---:|\n| 1 | 22 |\n", 40);
        assert!(table.contains("\u{250c}") && table.contains("\u{2502} a \u{2502} bb \u{2502}") && table.contains("\u{2502} 1 \u{2502} 22 \u{2502}"), "{table}");
        let task = show("- [x] done\n- [ ] todo\n", 40);
        assert!(task.contains("[x] done") && task.contains("[ ] todo"), "{task}");
    }

    #[test]
    fn links_are_marked_and_missing_images_show_their_alt() {
        let text = render("[site](https://a.io) ![a cat](nope.png)", 40, Path::new("."));
        assert!(text.contains("\x1b]8;;https://a.io\x1b\\"));
        assert!(plain(&text).contains("[a cat]"));
    }
}
