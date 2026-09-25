//! Shell-integration marks.
//!
//! KeyJutsu's prompt wrapper makes the shell emit FinalTerm/OSC 133 marks
//! around every prompt: `D` (the previous command finished, with its exit code
//! where the shell can report one), `A` (prompt starts) and `B` (prompt ends,
//! the input line begins). The execution engine uses these to know a command
//! has really finished instead of waiting for a timer to expire.
//!
//! Anything running inside the shell can print an OSC 133 sequence too, so
//! each mark KeyJutsu emits carries a per-session nonce (`kj=<nonce>`). Marks
//! without the right nonce are passed through as ordinary output and never
//! trusted. The nonce raises the bar rather than closing it: a process in the
//! session can read its own parent's command line. That limit is recorded in
//! THREAT_MODEL.md.

use std::fmt;

const OSC_133: &[u8] = b"\x1b]133;";
const CURSOR_QUERY: &[u8] = b"\x1b[6n";
/// A mark longer than this is not ours; stop buffering and pass it through.
/// Long enough for a location mark with a long Windows path.
const MAX_SEQUENCE: usize = 4096;

/// A per-session secret stamped on every mark KeyJutsu's integration emits.
#[derive(Clone, PartialEq, Eq)]
pub struct Nonce(String);

impl Nonce {
    /// 128 bits from the operating system's generator, hex encoded so it can
    /// be embedded in a PowerShell string or a cmd `PROMPT` without escaping.
    pub fn generate() -> crate::Result<Self> {
        let mut bytes = [0u8; 16];
        getrandom::fill(&mut bytes).map_err(|e| crate::TerminalError::Random(e.to_string()))?;
        Ok(Self(bytes.iter().map(|b| format!("{b:02x}")).collect()))
    }

    /// Only for tests and fixtures: production nonces come from [`Nonce::generate`].
    pub fn from_fixed(value: &str) -> Self {
        assert!(
            !value.is_empty() && value.bytes().all(|b| b.is_ascii_alphanumeric()),
            "a nonce must be non-empty ASCII alphanumerics"
        );
        Self(value.to_owned())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Nonce {
    // Kept out of logs: it is the only thing that separates a real mark from
    // one printed by a command.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Nonce(..)")
    }
}

/// A prompt mark reported by the shell integration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, ts_rs::TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export)]
pub enum ShellMark {
    /// `A`: the prompt is about to be drawn.
    PromptStart,
    /// `B`: the prompt has been drawn and the input line begins.
    CommandStart,
    /// `C`: a command has begun executing. Only shells that can hook
    /// pre-execution emit this; none of the V1 integrations do yet.
    CommandExecuted,
    /// `D`: the previous command finished. `exit_code` is `None` where the
    /// shell cannot report one, which is the case for `cmd.exe`.
    CommandFinished { exit_code: Option<i32> },
}

/// One piece of scanned terminal output, in stream order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScanItem {
    /// Bytes to show the user, with KeyJutsu's own marks removed.
    Output(Vec<u8>),
    Mark(ShellMark),
    /// KeyJutsu's own `P` mark: the shell's current folder, reported by the
    /// prompt, so KeyJutsu knows where commands will run.
    Location(String),
    /// ConPTY asked where the cursor is (`ESC [ 6 n`) and is waiting for an
    /// answer. Only reported when the scanner was asked to intercept these.
    CursorQuery,
}

/// Streaming scanner that splits pseudo-console output into display bytes and
/// trusted marks. Sequences split across reads are held until complete.
#[derive(Debug)]
pub struct MarkScanner {
    nonce: Nonce,
    intercept_cursor_query: bool,
    pending: Vec<u8>,
}

impl MarkScanner {
    pub fn new(nonce: Nonce) -> Self {
        Self { nonce, intercept_cursor_query: false, pending: Vec::new() }
    }

    /// Report `ESC [ 6 n` as [`ScanItem::CursorQuery`] instead of passing it
    /// through. A renderer such as xterm.js answers the query itself; a
    /// headless session or a console pass-through has to answer it for the
    /// shell, because ConPTY blocks until it gets a reply.
    pub fn intercept_cursor_queries(mut self, yes: bool) -> Self {
        self.intercept_cursor_query = yes;
        self
    }

    pub fn feed(&mut self, data: &[u8]) -> Vec<ScanItem> {
        let mut buf = std::mem::take(&mut self.pending);
        buf.extend_from_slice(data);

        let mut items = Vec::new();
        let mut out = Vec::new();
        let mut i = 0;
        while i < buf.len() {
            if buf[i] != 0x1b {
                out.push(buf[i]);
                i += 1;
                continue;
            }
            let rest = &buf[i..];

            if self.intercept_cursor_query {
                if rest.starts_with(CURSOR_QUERY) {
                    flush(&mut out, &mut items);
                    items.push(ScanItem::CursorQuery);
                    i += CURSOR_QUERY.len();
                    continue;
                }
                if CURSOR_QUERY.starts_with(rest) {
                    self.pending = rest.to_vec();
                    break;
                }
            }

            if rest.len() < OSC_133.len() && OSC_133.starts_with(rest) {
                self.pending = rest.to_vec();
                break;
            }
            if rest.starts_with(OSC_133) {
                match find_terminator(&rest[OSC_133.len()..]) {
                    Some((body_len, term_len)) => {
                        let body = &rest[OSC_133.len()..OSC_133.len() + body_len];
                        let total = OSC_133.len() + body_len + term_len;
                        match parse_mark(body, &self.nonce) {
                            Some(item) => {
                                flush(&mut out, &mut items);
                                items.push(item);
                            }
                            // Not ours: show it exactly as it arrived.
                            None => out.extend_from_slice(&rest[..total]),
                        }
                        i += total;
                        continue;
                    }
                    None if rest.len() <= MAX_SEQUENCE => {
                        self.pending = rest.to_vec();
                        break;
                    }
                    None => {}
                }
            }

            out.push(buf[i]);
            i += 1;
        }
        flush(&mut out, &mut items);
        items
    }

    /// Release anything still held back, for use when the stream has ended.
    pub fn finish(&mut self) -> Vec<ScanItem> {
        let pending = std::mem::take(&mut self.pending);
        if pending.is_empty() { Vec::new() } else { vec![ScanItem::Output(pending)] }
    }
}

fn flush(out: &mut Vec<u8>, items: &mut Vec<ScanItem>) {
    if !out.is_empty() {
        items.push(ScanItem::Output(std::mem::take(out)));
    }
}

/// Length of the body and of its terminator (BEL, or ESC `\`).
fn find_terminator(s: &[u8]) -> Option<(usize, usize)> {
    for (idx, &b) in s.iter().enumerate() {
        match b {
            0x07 => return Some((idx, 1)),
            0x1b if s.get(idx + 1) == Some(&b'\\') => return Some((idx, 2)),
            _ => {}
        }
    }
    None
}

fn parse_mark(body: &[u8], nonce: &Nonce) -> Option<ScanItem> {
    let body = std::str::from_utf8(body).ok()?;
    // `P;kj=<nonce>;cwd=<path>`: the path runs to the end, so a `;` in a
    // folder name does not split it.
    if let Some(rest) = body.strip_prefix("P;kj=") {
        let (n, path) = rest.split_once(";cwd=")?;
        return (n == nonce.as_str() && !path.is_empty()).then(|| ScanItem::Location(path.to_owned()));
    }
    let mut parts = body.split(';');
    let kind = parts.next()?;
    let mut nonce_ok = false;
    let mut first_value: Option<&str> = None;
    for part in parts {
        match part.strip_prefix("kj=") {
            Some(v) => nonce_ok = v == nonce.as_str(),
            None if first_value.is_none() => first_value = Some(part),
            None => {}
        }
    }
    if !nonce_ok {
        return None;
    }
    Some(ScanItem::Mark(match kind {
        "A" => ShellMark::PromptStart,
        "B" => ShellMark::CommandStart,
        "C" => ShellMark::CommandExecuted,
        "D" => ShellMark::CommandFinished { exit_code: first_value.and_then(|v| v.parse().ok()) },
        _ => return None,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scanner() -> MarkScanner {
        MarkScanner::new(Nonce::from_fixed("n0nce"))
    }

    fn output_of(items: &[ScanItem]) -> Vec<u8> {
        items
            .iter()
            .filter_map(|i| match i {
                ScanItem::Output(b) => Some(b.clone()),
                _ => None,
            })
            .flatten()
            .collect()
    }

    fn marks_of(items: &[ScanItem]) -> Vec<ShellMark> {
        items
            .iter()
            .filter_map(|i| match i {
                ScanItem::Mark(m) => Some(*m),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn recognises_marks_with_the_session_nonce_and_strips_them() {
        // Captured from pwsh 7.6 through ConPTY on Windows build 26200.
        let raw = b"hi\r\n\x1b]133;D;0;kj=n0nce\x07\x1b]133;A;kj=n0nce\x07PS> \x1b]133;B;kj=n0nce\x07";
        let items = scanner().feed(raw);
        assert_eq!(
            marks_of(&items),
            vec![
                ShellMark::CommandFinished { exit_code: Some(0) },
                ShellMark::PromptStart,
                ShellMark::CommandStart
            ]
        );
        assert_eq!(output_of(&items), b"hi\r\nPS> ");
    }

    #[test]
    fn accepts_the_string_terminator_form_cmd_uses() {
        let raw = b"\x1b]133;D;kj=n0nce\x1b\\C:\\>\x1b]133;B;kj=n0nce\x1b\\";
        let items = scanner().feed(raw);
        assert_eq!(
            marks_of(&items),
            vec![ShellMark::CommandFinished { exit_code: None }, ShellMark::CommandStart]
        );
        assert_eq!(output_of(&items), b"C:\\>");
    }

    #[test]
    fn reports_non_zero_exit_codes() {
        let items = scanner().feed(b"\x1b]133;D;3;kj=n0nce\x07");
        assert_eq!(marks_of(&items), vec![ShellMark::CommandFinished { exit_code: Some(3) }]);
    }

    #[test]
    fn a_mark_with_the_wrong_nonce_is_passed_through_untrusted() {
        let spoof = b"\x1b]133;D;0;kj=guess\x07";
        let items = scanner().feed(spoof);
        assert!(marks_of(&items).is_empty());
        assert_eq!(output_of(&items), spoof);
    }

    #[test]
    fn a_mark_with_no_nonce_is_passed_through_untrusted() {
        let foreign = b"\x1b]133;A\x07";
        let items = scanner().feed(foreign);
        assert!(marks_of(&items).is_empty());
        assert_eq!(output_of(&items), foreign);
    }

    #[test]
    fn a_mark_split_across_every_possible_read_boundary_is_still_found() {
        let raw: &[u8] = b"ab\x1b]133;D;7;kj=n0nce\x07cd";
        for split in 0..=raw.len() {
            let mut s = scanner();
            let mut items = s.feed(&raw[..split]);
            items.extend(s.feed(&raw[split..]));
            items.extend(s.finish());
            assert_eq!(
                marks_of(&items),
                vec![ShellMark::CommandFinished { exit_code: Some(7) }],
                "split at {split}"
            );
            assert_eq!(output_of(&items), b"abcd", "split at {split}");
        }
    }

    #[test]
    fn other_escape_sequences_are_untouched() {
        let raw = b"\x1b[93mecho\x1b[0m \x1b]0;title\x07\x1b[?25h";
        let items = scanner().feed(raw);
        assert_eq!(output_of(&items), raw);
    }

    #[test]
    fn an_unterminated_sequence_is_released_once_it_is_too_long_to_be_ours() {
        let mut raw = b"\x1b]133;A;".to_vec();
        raw.extend(std::iter::repeat_n(b'x', MAX_SEQUENCE + 10));
        let items = scanner().feed(&raw);
        assert_eq!(output_of(&items), raw);
    }

    #[test]
    fn cursor_queries_pass_through_unless_intercepted() {
        let raw = b"\x1b[6n\x1b[?9001h";
        assert_eq!(output_of(&scanner().feed(raw)), raw);

        let items = scanner().intercept_cursor_queries(true).feed(raw);
        assert_eq!(items[0], ScanItem::CursorQuery);
        assert_eq!(output_of(&items), b"\x1b[?9001h");
    }

    #[test]
    fn an_intercepted_cursor_query_split_across_reads_is_found() {
        let mut s = scanner().intercept_cursor_queries(true);
        let mut items = s.feed(b"x\x1b[");
        items.extend(s.feed(b"6ny"));
        assert!(items.contains(&ScanItem::CursorQuery));
        assert_eq!(output_of(&items), b"xy");
    }

    #[test]
    fn the_nonce_never_appears_in_debug_output() {
        let n = Nonce::from_fixed("secretvalue");
        assert!(!format!("{n:?}").contains("secretvalue"));
    }

    #[test]
    fn generated_nonces_are_32_hex_characters_and_differ() {
        let a = Nonce::generate().unwrap();
        let b = Nonce::generate().unwrap();
        assert_eq!(a.as_str().len(), 32);
        assert!(a.as_str().bytes().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }

    #[test]
    fn the_shell_reports_its_folder_even_with_semicolons_in_it() {
        let items = scanner().feed(b"x\x1b]133;P;kj=n0nce;cwd=C:\\Work;Files\\a b\x07y");
        assert!(items.contains(&ScanItem::Location("C:\\Work;Files\\a b".into())), "{items:?}");
        assert_eq!(output_of(&items), b"xy");
        let spoof = scanner().feed(b"\x1b]133;P;kj=guess;cwd=C:\\Evil\x07");
        assert!(!spoof.iter().any(|i| matches!(i, ScanItem::Location(_))), "a wrong nonce is only output");
    }
}
