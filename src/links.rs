//! Links in the terminal, opened with Cmd+click: hyperlinks programs mark
//! (OSC 8, as `ls --hyperlink` and some compilers send), web addresses in
//! the text, and names of files and folders that exist. Holding Cmd
//! underlines the one under the pointer.
//!
//! Text is read a logical line at a time, rows joined where the terminal
//! wrapped them, so an address broken over two rows is still one link.

use std::path::{Path, PathBuf};

use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line, Point};
use alacritty_terminal::term::Term;
use alacritty_terminal::term::cell::Flags;

/// What a link opens.
#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    Url(String),
    Path(PathBuf),
}

/// A link: what it opens and the cells it covers, a run per row (grid
/// line, first column, number of columns).
#[derive(Clone, Debug, PartialEq)]
pub struct Link {
    pub target: Target,
    pub spans: Vec<(i32, usize, usize)>,
}

/// Schemes recognised in plain text.
const SCHEMES: [&str; 7] = ["https://", "http://", "file://", "ftp://", "ssh://", "git://", "mailto:"];

/// The link at `point`, if there is one. Words are looked up as paths
/// from `cwd` (the shell's working directory).
pub fn at<L>(term: &Term<L>, point: Point, cwd: Option<&Path>) -> Option<Link> {
    let grid = term.grid();
    if point.line < grid.topmost_line() || point.line > grid.bottommost_line() || point.column.0 >= grid.columns() {
        return None;
    }
    let cells = logical_line(term, point.line);
    let here = cells.iter().position(|c| c.point == point)?;

    // A hyperlink the program marked: every cell of it along the line.
    if let Some(uri) = cells[here].uri.clone() {
        let (mut a, mut b) = (here, here);
        while a > 0 && cells[a - 1].uri.as_ref() == Some(&uri) {
            a -= 1;
        }
        while b + 1 < cells.len() && cells[b + 1].uri.as_ref() == Some(&uri) {
            b += 1;
        }
        return Some(Link { target: Target::Url(uri), spans: spans(&cells[a..=b]) });
    }

    let text: Vec<char> = cells.iter().map(|c| c.c).collect();
    if let Some((a, b)) = urls(&text).into_iter().find(|&(a, b)| (a..b).contains(&here)) {
        let url: String = text[a..b].iter().collect();
        return Some(Link { target: Target::Url(url), spans: spans(&cells[a..b]) });
    }
    let (a, b) = word(&text, here)?;
    let name: String = text[a..b].iter().collect();
    let path = existing(&name, cwd?)?;
    Some(Link { target: Target::Path(path), spans: spans(&cells[a..b]) })
}

/// Opens a link with the system: the browser, mail, the Finder or the
/// file's own app.
pub fn open(target: &Target) -> std::io::Result<()> {
    let mut command = std::process::Command::new(if cfg!(target_os = "macos") { "open" } else { "xdg-open" });
    match target {
        Target::Url(url) => command.arg(url),
        Target::Path(path) => command.arg(path),
    };
    command.spawn().map(|_| ())
}

struct Cell {
    c: char,
    point: Point,
    uri: Option<String>,
}

/// The cells of the logical line through `line`: the rows before it that
/// wrapped into it, it, and the rows it wraps into. Wide characters count
/// once.
fn logical_line<L>(term: &Term<L>, line: Line) -> Vec<Cell> {
    let grid = term.grid();
    let columns = grid.columns();
    let wraps = |l: Line| grid[l][Column(columns - 1)].flags.contains(Flags::WRAPLINE);
    let mut first = line;
    while first > grid.topmost_line() && wraps(first - 1) {
        first -= 1;
    }
    let mut last = line;
    while last < grid.bottommost_line() && wraps(last) {
        last += 1;
    }
    let mut cells = Vec::new();
    let mut l = first;
    while l <= last {
        for col in 0..columns {
            let cell = &grid[l][Column(col)];
            if cell.flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
                continue;
            }
            let c = if cell.c == '\0' { ' ' } else { cell.c };
            // Prompts and pictures carry links of the app's own (marks.rs,
            // images.rs); they open nothing.
            let uri = cell
                .hyperlink()
                .map(|h| h.uri().to_string())
                .filter(|u| !u.starts_with(crate::marks::PROMPT_URI) && !u.starts_with(crate::images::IMAGE_URI));
            cells.push(Cell { c, point: Point::new(l, Column(col)), uri });
        }
        l += 1;
    }
    cells
}

/// Cells as runs per row.
fn spans(cells: &[Cell]) -> Vec<(i32, usize, usize)> {
    let mut out: Vec<(i32, usize, usize)> = Vec::new();
    for cell in cells {
        let (line, col) = (cell.point.line.0, cell.point.column.0);
        match out.last_mut() {
            Some((l, start, len)) if *l == line && *start + *len <= col => *len = col + 1 - *start,
            _ => out.push((line, col, 1)),
        }
    }
    out
}

/// Web addresses in `text`, as ranges: from a known scheme to the first
/// space or character that cannot be part of one, without trailing
/// punctuation or a closing bracket that was not opened inside it.
fn urls(text: &[char]) -> Vec<(usize, usize)> {
    let stops = |c: char| c.is_whitespace() || c.is_control() || "<>\"'`{}|\\^".contains(c);
    let mut out = Vec::new();
    let mut i = 0;
    while i < text.len() {
        let rest: String = text[i..text.len().min(i + 9)].iter().collect();
        let starts = SCHEMES.iter().find(|s| rest.to_ascii_lowercase().starts_with(*s));
        let after_word = i > 0 && text[i - 1].is_alphanumeric();
        let Some(scheme) = starts.filter(|_| !after_word) else {
            i += 1;
            continue;
        };
        let mut end = i;
        while end < text.len() && !stops(text[end]) {
            end += 1;
        }
        while let Some(&last) = text[i..end].last() {
            let unbalanced = |open: char, close: char| {
                last == close && text[i..end].iter().filter(|&&c| c == open).count() < text[i..end].iter().filter(|&&c| c == close).count()
            };
            if ".,;:!?".contains(last) || unbalanced('(', ')') || unbalanced('[', ']') {
                end -= 1;
            } else {
                break;
            }
        }
        if end > i + scheme.chars().count() {
            out.push((i, end));
        }
        i = end.max(i + 1);
    }
    out
}

/// The word around `at` that could name a file: between spaces, quotes and
/// brackets, without trailing punctuation or the marks `ls -F` adds.
fn word(text: &[char], at: usize) -> Option<(usize, usize)> {
    let breaks = |c: char| c.is_whitespace() || "\"'`()[]{}<>|;,=".contains(c);
    if breaks(text[at]) {
        return None;
    }
    let mut a = at;
    while a > 0 && !breaks(text[a - 1]) {
        a -= 1;
    }
    let mut b = at + 1;
    while b < text.len() && !breaks(text[b]) {
        b += 1;
    }
    while b > a && "*@:.,".contains(text[b - 1]) {
        b -= 1;
    }
    (b > at).then_some((a, b))
}

/// The file or folder `name` names, from `cwd` or home, if it exists. A
/// `:line` or `:line:column` after a file name (as compilers print) is
/// not part of it.
fn existing(name: &str, cwd: &Path) -> Option<PathBuf> {
    let mut name = name;
    for _ in 0..2 {
        if let Some((head, tail)) = name.rsplit_once(':')
            && !tail.is_empty()
            && tail.chars().all(|c| c.is_ascii_digit())
        {
            name = head;
        }
    }
    if name.is_empty() || name.starts_with('-') || name == "." || name == ".." {
        return None;
    }
    let path = match name.strip_prefix("~/") {
        Some(rest) => PathBuf::from(std::env::var_os("HOME")?).join(rest),
        None => cwd.join(name),
    };
    (path != cwd && path.symlink_metadata().is_ok()).then_some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(text: &str) -> Vec<String> {
        let chars: Vec<char> = text.chars().collect();
        urls(&chars).into_iter().map(|(a, b)| chars[a..b].iter().collect()).collect()
    }

    #[test]
    fn addresses_end_where_the_text_around_them_starts() {
        assert_eq!(found("see https://example.com/a?b=c."), ["https://example.com/a?b=c"]);
        assert_eq!(found("(docs at https://en.wikipedia.org/wiki/Ink_(disambiguation))"), [
            "https://en.wikipedia.org/wiki/Ink_(disambiguation)"
        ]);
        assert_eq!(found("<http://a.io/x>, mailto:me@a.io"), ["http://a.io/x", "mailto:me@a.io"]);
        assert_eq!(found("\"file:///tmp/x y\""), ["file:///tmp/x"]);
        assert!(found("nothttps://a.io https:// plain text").is_empty());
    }

    /// A 20-column terminal that has been sent `bytes`.
    fn term(bytes: &[u8]) -> Term<alacritty_terminal::event::VoidListener> {
        use alacritty_terminal::vte::ansi::{Processor, StdSyncHandler};
        let size = crate::terminal::GridSize { columns: 20, lines: 5, cell_width: 8.0, cell_height: 16.0 };
        let mut term = Term::new(alacritty_terminal::term::Config::default(), &size, alacritty_terminal::event::VoidListener);
        let mut parser: Processor<StdSyncHandler> = Processor::new();
        parser.advance(&mut term, bytes);
        term
    }

    #[test]
    fn links_follow_wrapped_rows_and_marked_hyperlinks() {
        // "go https://example.com/" wraps after 20 columns.
        let t = term(b"go https://example.com/long/path ok");
        let link = at(&t, Point::new(Line(1), Column(2)), None).unwrap();
        assert_eq!(link.target, Target::Url("https://example.com/long/path".into()));
        assert_eq!(link.spans, vec![(0, 3, 17), (1, 0, 12)]);
        assert!(at(&t, Point::new(Line(0), Column(0)), None).is_none());

        let t = term(b"\x1b]8;;https://a.io/doc\x1b\\read me\x1b]8;;\x1b\\ later");
        let link = at(&t, Point::new(Line(0), Column(5)), None).unwrap();
        assert_eq!(link.target, Target::Url("https://a.io/doc".into()));
        assert_eq!(link.spans, vec![(0, 0, 7)]);
        assert!(at(&t, Point::new(Line(0), Column(9)), None).is_none());
    }

    #[test]
    fn names_of_files_that_exist_are_links() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let text: Vec<char> = "error at src/links.rs:12:5: oops".chars().collect();
        let (a, b) = word(&text, 12).unwrap();
        let name: String = text[a..b].iter().collect();
        assert_eq!(name, "src/links.rs:12:5");
        assert_eq!(existing(&name, root), Some(root.join("src/links.rs")));
        assert_eq!(existing("src/nope.rs", root), None);
        assert_eq!(existing(".", root), None);
        assert!(word(&text, 2).is_some_and(|(a, b)| text[a..b].iter().collect::<String>() == "error"));
        assert_eq!(word(&text, 8), None);
    }
}
