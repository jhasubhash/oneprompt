//! Find (Cmd+F): every match of the text typed, in the scrollback and on
//! screen, highlighted, with one of them current. Return (or Cmd+G, or ↑)
//! goes to the one before it, older; Shift+Return (Cmd+Shift+G, ↓) to the
//! next, newer. The view scrolls to show it. Lowercase text matches either
//! case; any capital makes the search exact. Cmd+R switches between the
//! text taken literally and taken as a regular expression.
//!
//! Matches are found by alacritty_terminal's search. Grid lines move as output scrolls the screen, so the current
//! match is remembered by its place counted from the oldest line, which
//! new output does not change.

use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Direction, Point};
use alacritty_terminal::term::Term;
use alacritty_terminal::term::search::{Match, RegexIter, RegexSearch};

/// Matches beyond this many are not found: the count says "1000+".
pub const MAX_MATCHES: usize = 1000;

#[derive(Default)]
pub struct Search {
    pub query: String,
    pub matches: Vec<Match>,
    pub current: Option<usize>,
    /// The query is a regular expression, not text to match as it is.
    pub regex: bool,
    /// The query is a regular expression that does not parse.
    pub invalid: bool,
    /// The scrollback's length when the matches were found: a match's line
    /// plus this is its place from the oldest line.
    history: usize,
}

impl Search {
    /// Finds the matches again (the text or the terminal changed), keeping
    /// the current one on the same text if it is still there; otherwise the
    /// newest one at or above the bottom of the view.
    pub fn refresh<T>(&mut self, term: &Term<T>) {
        let old = self.current.and_then(|i| self.matches.get(i)).map(|m| (m.start().line.0 + self.history as i32, m.start().column));
        let grid = term.grid();
        self.history = grid.history_size();
        match find(term, &self.query, self.regex) {
            Some(matches) => {
                self.matches = matches;
                self.invalid = false;
            }
            None => {
                self.matches.clear();
                self.invalid = true;
            }
        }
        let here = |m: &Match| (m.start().line.0 + self.history as i32, m.start().column);
        self.current = match old {
            Some(old) => self.matches.iter().position(|m| here(m) >= old).or(self.matches.len().checked_sub(1)),
            None => {
                let bottom = grid.screen_lines() as i32 - 1 - grid.display_offset() as i32;
                self.matches.iter().rposition(|m| m.start().line.0 <= bottom).or(self.matches.len().checked_sub(1))
            }
        };
    }

    /// The text changed: starts over from the bottom of the view.
    pub fn set_query<T>(&mut self, term: &Term<T>, query: String) {
        self.query = query;
        self.current = None;
        self.refresh(term);
    }

    /// Switches between literal text and a regular expression, keeping
    /// the query.
    pub fn toggle_regex<T>(&mut self, term: &Term<T>) {
        self.regex = !self.regex;
        self.current = None;
        self.refresh(term);
    }

    /// Goes to the match before the current one (`older`) or after it,
    /// round from the end to the start.
    pub fn step(&mut self, older: bool) {
        let n = self.matches.len();
        if n == 0 {
            return;
        }
        self.current = Some(match self.current {
            Some(i) if older => (i + n - 1) % n,
            Some(i) => (i + 1) % n,
            None => n - 1,
        });
    }

    /// How far to scroll the view (as `Scroll::Delta`) to bring the current
    /// match into it, centred; 0 if it is in view.
    pub fn reveal<T>(&self, term: &Term<T>) -> i32 {
        let Some(m) = self.current.and_then(|i| self.matches.get(i)) else { return 0 };
        let grid = term.grid();
        let (lines, offset) = (grid.screen_lines() as i32, grid.display_offset() as i32);
        let line = m.start().line.0;
        if (-offset..lines - offset).contains(&line) {
            return 0;
        }
        let wanted = (lines / 2 - line).clamp(0, grid.history_size() as i32);
        wanted - offset
    }

    /// The matches in view, a run per row: screen line, first column,
    /// length, and whether it is the current match.
    pub fn marks<T>(&self, term: &Term<T>) -> Vec<(usize, usize, usize, bool)> {
        let grid = term.grid();
        let (lines, columns, offset) = (grid.screen_lines() as i32, grid.columns(), grid.display_offset() as i32);
        let mut out = Vec::new();
        for (i, m) in self.matches.iter().enumerate() {
            let (start, end) = (*m.start(), *m.end());
            if end.line.0 + offset < 0 || start.line.0 + offset >= lines {
                continue;
            }
            for line in start.line.0..=end.line.0 {
                let screen = line + offset;
                if !(0..lines).contains(&screen) {
                    continue;
                }
                let first = if line == start.line.0 { start.column.0 } else { 0 };
                let last = if line == end.line.0 { end.column.0 } else { columns - 1 };
                out.push((screen as usize, first, last + 1 - first, Some(i) == self.current));
            }
        }
        out
    }

    /// "3 of 12", or None with nothing typed.
    pub fn count(&self) -> Option<(usize, usize)> {
        (!self.query.is_empty()).then(|| (self.current.map_or(0, |i| i + 1), self.matches.len()))
    }

    /// The current match, to leave selected when the search closes.
    pub fn current_match(&self) -> Option<Match> {
        self.current.and_then(|i| self.matches.get(i)).cloned()
    }
}

/// Every match of `query`, oldest first: taken literally, or as a regular
/// expression with `regex`. None if the expression does not parse.
fn find<T>(term: &Term<T>, query: &str, regex: bool) -> Option<Vec<Match>> {
    if query.is_empty() {
        return Some(Vec::new());
    }
    let pattern = if regex {
        query.to_string()
    } else {
        let mut pattern = String::new();
        for c in query.chars() {
            if "\\.+*?()|[]{}^$#&-~".contains(c) {
                pattern.push('\\');
            }
            pattern.push(c);
        }
        pattern
    };
    let mut regex = RegexSearch::new(&pattern).ok()?;
    let grid = term.grid();
    let start = Point::new(grid.topmost_line(), Column(0));
    let end = Point::new(grid.bottommost_line(), Column(grid.columns() - 1));
    Some(RegexIter::new(start, end, Direction::Right, term, &mut regex).take(MAX_MATCHES).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use alacritty_terminal::event::VoidListener;
    use alacritty_terminal::grid::Scroll;
    use alacritty_terminal::vte::ansi::{Processor, StdSyncHandler};

    fn term(text: &str) -> Term<VoidListener> {
        let size = crate::terminal::GridSize { columns: 20, lines: 4, cell_width: 8.0, cell_height: 16.0 };
        let mut term = Term::new(alacritty_terminal::term::Config::default(), &size, VoidListener);
        let mut parser: Processor<StdSyncHandler> = Processor::new();
        parser.advance(&mut term, text.replace('\n', "\r\n").as_bytes());
        term
    }

    #[test]
    fn finds_text_literally_and_starts_at_the_newest() {
        let mut t = term("a.b one\nA.B two\naxb three");
        let mut search = Search::default();
        search.set_query(&t, "a.b".into());
        // Lowercase matches either case, and "." is a dot.
        assert_eq!(search.count(), Some((2, 2)));
        assert_eq!(search.marks(&t), vec![(0, 0, 3, false), (1, 0, 3, true)]);
        search.step(true);
        assert_eq!(search.count(), Some((1, 2)));
        search.step(true);
        assert_eq!(search.count(), Some((2, 2)));
        search.set_query(&t, "A.B".into());
        assert_eq!(search.count(), Some((1, 1)));
        search.set_query(&t, "zzz".into());
        assert_eq!(search.count(), Some((0, 0)));

        // A match scrolled off is brought back into view.
        search.set_query(&t, "one".into());
        let mut parser: Processor<StdSyncHandler> = Processor::new();
        parser.advance(&mut t, b"\r\n1\r\n2\r\n3\r\n4\r\n5");
        search.refresh(&t);
        assert_eq!(search.count(), Some((1, 1)));
        let delta = search.reveal(&t);
        assert!(delta > 0);
        t.scroll_display(Scroll::Delta(delta));
        assert_eq!(search.reveal(&t), 0);
        assert_eq!(search.marks(&t).len(), 1);
    }

    #[test]
    fn regular_expressions_match_patterns_and_bad_ones_say_so() {
        let t = term("a.b one\nA.B two\naxb three");
        let mut search = Search::default();
        search.set_query(&t, "a.b".into());
        assert_eq!(search.count(), Some((2, 2)));
        // As a regular expression "." is any character, so axb matches too.
        search.toggle_regex(&t);
        assert_eq!(search.count(), Some((3, 3)));
        search.set_query(&t, "t(wo|hree)".into());
        assert_eq!(search.count(), Some((2, 2)));
        assert!(!search.invalid);
        search.set_query(&t, "(one".into());
        assert!(search.invalid);
        assert_eq!(search.count(), Some((0, 0)));
        // Back to text: the same query is just text again.
        search.toggle_regex(&t);
        assert!(!search.invalid);
    }
}
