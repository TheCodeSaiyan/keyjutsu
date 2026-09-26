//! What a shell printed, as a terminal shows it.
//!
//! Output is not text: it moves the cursor, erases, and rewrites. The line
//! editor in PowerShell redraws the input line as it is typed, by moving the
//! cursor back and writing the line again, so removing the escape codes and
//! keeping the rest shows the command twice, run together. Where output is
//! read as text (a failed step's output for the operator and the agent, and
//! what an Administrator step printed in the broker's shell), it is rendered
//! on a screen the size of the pseudo-console, with scrollback, and read
//! back line by line.

/// Lines kept above the screen. More than a step's output is ever read back
/// for; once exceeded, the oldest go.
const SCROLLBACK: usize = 10_000;

pub struct Transcript {
    parser: vt100::Parser,
}

impl std::fmt::Debug for Transcript {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Transcript").field("size", &self.parser.screen().size()).finish_non_exhaustive()
    }
}

impl Transcript {
    pub fn new(rows: u16, cols: u16) -> Self {
        Self { parser: vt100::Parser::new(rows.max(1), cols.max(1), SCROLLBACK) }
    }

    pub fn feed(&mut self, text: &str) {
        self.parser.process(text.as_bytes());
    }

    pub fn resize(&mut self, rows: u16, cols: u16) {
        self.parser.screen_mut().set_size(rows.max(1), cols.max(1));
    }

    /// How many lines have scrolled above the screen.
    fn depth(&mut self) -> usize {
        let screen = self.parser.screen_mut();
        screen.set_scrollback(usize::MAX);
        let depth = screen.scrollback();
        screen.set_scrollback(0);
        depth
    }

    /// The line the cursor is on, counted from the top of the scrollback: a
    /// place to read from later.
    pub fn mark(&mut self) -> usize {
        self.depth() + usize::from(self.parser.screen().cursor_position().0)
    }

    /// Every line, scrollback first, down to the cursor's. A line the
    /// terminal wrapped is joined back into one.
    fn lines(&mut self) -> Vec<String> {
        let (rows, cols) = self.parser.screen().size();
        let rows = usize::from(rows);
        let depth = self.depth();
        let mut out: Vec<String> = Vec::new();
        let mut joining = false;
        let mut push = |text: String, wrapped: bool, out: &mut Vec<String>| {
            if joining && let Some(last) = out.last_mut() {
                last.push_str(&text);
            } else {
                out.push(text);
            }
            joining = wrapped;
        };
        // The scrollback, a screenful at a time from the oldest.
        let mut offset = depth;
        while offset > 0 {
            let screen = self.parser.screen_mut();
            screen.set_scrollback(offset);
            let take = offset.min(rows);
            let texts: Vec<String> = screen.rows(0, cols).take(take).collect();
            for (i, text) in texts.into_iter().enumerate() {
                let wrapped = self.parser.screen().row_wrapped(u16::try_from(i).unwrap_or(u16::MAX));
                push(text, wrapped, &mut out);
            }
            offset -= take;
        }
        let screen = self.parser.screen_mut();
        screen.set_scrollback(0);
        let cursor_row = usize::from(screen.cursor_position().0);
        let texts: Vec<String> = screen.rows(0, cols).take(cursor_row + 1).collect();
        for (i, text) in texts.into_iter().enumerate() {
            let wrapped = self.parser.screen().row_wrapped(u16::try_from(i).unwrap_or(u16::MAX));
            push(text, wrapped, &mut out);
        }
        out
    }

    /// The lines from `mark` on, as a person would read them, and no more
    /// than the last `max_chars` characters of them.
    pub fn since(&mut self, mark: usize, max_chars: usize) -> String {
        let lines = self.lines();
        let from = mark.min(lines.len());
        let text: String = lines[from..].iter().map(|l| l.trim_end()).collect::<Vec<_>>().join("\n");
        let text = text.trim_end().to_owned();
        let skip = text.chars().count().saturating_sub(max_chars);
        text.chars().skip(skip).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::Transcript;

    /// Windows PowerShell 5.1 echoing a command written in one go: its line
    /// editor draws part of it, then moves the cursor back to column 20 and
    /// draws it all again. Captured from a real session.
    const REDRAWN: &str = "\u{1b}[93mWrite-Output \u{1b}[36m'hello from the step'\u{1b}[37m; \u{1b}[93mGet-Date \u{1b}[90m-Format \u{1b}[37my\u{1b}[?25l\u{1b}[m\u{1b}[93m\u{1b}[1;20HWrite-Output \u{1b}[36m'hello from the step'\u{1b}[37m; \u{1b}[93mGet-Date \u{1b}[90m-Format \u{1b}[37myyyy\r\n\u{1b}[?25h\u{1b}[mhello from the step\r\n2026\r\nPS C:\\Users\\robin> ";

    #[test]
    fn a_line_the_shell_redrew_reads_once() {
        let mut t = Transcript::new(30, 120);
        t.feed("PS C:\\Users\\robin> ");
        let mark = t.mark();
        t.feed(REDRAWN);
        assert_eq!(
            t.since(mark, 10_000),
            "PS C:\\Users\\robin> Write-Output 'hello from the step'; Get-Date -Format yyyy\n\
             hello from the step\n\
             2026\n\
             PS C:\\Users\\robin>"
        );
        // The same stream with its codes merely removed, as before:
        let stripped = crate::headless::strip_ansi(REDRAWN);
        assert!(stripped.contains("-Format yWrite-Output"), "{stripped}");
    }

    #[test]
    fn a_mark_survives_the_screen_scrolling_and_the_tail_is_bounded() {
        let mut t = Transcript::new(5, 40);
        for i in 0..20 {
            t.feed(&format!("before {i}\r\n"));
        }
        let mark = t.mark();
        for i in 0..30 {
            t.feed(&format!("after {i}\r\n"));
        }
        let text = t.since(mark, 10_000);
        assert!(text.starts_with("after 0\n"), "{text}");
        assert!(text.ends_with("after 29"), "{text}");
        assert!(!text.contains("before"), "{text}");
        assert_eq!(t.since(mark, 8), "after 29");
    }

    #[test]
    fn a_line_the_terminal_wrapped_is_one_line() {
        let mut t = Transcript::new(10, 10);
        let mark = t.mark();
        t.feed("abcdefghijklmnopqrstuvwxy\r\nnext");
        assert_eq!(t.since(mark, 1000), "abcdefghijklmnopqrstuvwxy\nnext");
    }
}
