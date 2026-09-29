//! Recording a run: what the terminal drew, when, and where each step began
//! and ended (ADR 0021). Recorded only when the operator asks, kept with the
//! run's history, encrypted like it, and exported later in whatever cut and
//! form the operator chooses.
//!
//! The format is asciicast v2, asciinema's: a header, then one event per
//! line, `[seconds, "o", text]` for output and `[seconds, "m", label]` for a
//! marker. KeyJutsu's markers are `start:<step>` and `end:<step>:ok` or
//! `end:<step>:failed`.

use std::sync::Mutex;
use std::time::Instant;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// The longest pause kept when exporting: a slow download becomes two
/// seconds of stillness, not a minute.
pub const IDLE_LIMIT: f64 = 2.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "recording/")]
pub enum EventKind {
    Output,
    Marker,
    /// The terminal changed size: `COLSxROWS`, as asciicast writes it.
    Resize,
}

/// The size a resize event gives, as `(cols, rows)`.
pub fn size_of(data: &str) -> Option<(u16, u16)> {
    let (cols, rows) = data.split_once('x')?;
    Some((cols.trim().parse().ok()?, rows.trim().parse().ok()?))
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "recording/")]
pub struct Event {
    /// Seconds from the start.
    pub at: f64,
    pub kind: EventKind,
    pub data: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "recording/")]
pub struct Recording {
    pub width: u16,
    pub height: u16,
    pub title: String,
    pub events: Vec<Event>,
}

/// One step's place in a recording.
#[derive(Debug, Clone, PartialEq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "recording/")]
pub struct StepSpan {
    pub step: String,
    pub start: f64,
    /// `None` if the recording ends before the step does.
    pub end: Option<f64>,
    pub succeeded: Option<bool>,
}

/// Records while a run goes. Shared between the thread that sees output and
/// the one that sees steps start and finish.
#[derive(Debug)]
pub struct Recorder {
    started: Instant,
    width: u16,
    height: u16,
    events: Mutex<Vec<Event>>,
}

impl Recorder {
    pub fn new(width: u16, height: u16) -> Self {
        Self { started: Instant::now(), width, height, events: Mutex::new(Vec::new()) }
    }

    fn push(&self, kind: EventKind, data: String) {
        let at = self.started.elapsed().as_secs_f64();
        if let Ok(mut events) = self.events.lock() {
            events.push(Event { at, kind, data });
        }
    }

    pub fn output(&self, text: &str) {
        if !text.is_empty() {
            self.push(EventKind::Output, text.to_owned());
        }
    }

    pub fn step_started(&self, step: &str) {
        self.push(EventKind::Marker, format!("start:{step}"));
    }

    /// The terminal is now `cols` by `rows`: everything after is drawn for
    /// that size, so a replay must change size here too.
    pub fn resize(&self, cols: u16, rows: u16) {
        self.push(EventKind::Resize, format!("{cols}x{rows}"));
    }

    pub fn step_finished(&self, step: &str, succeeded: bool) {
        self.push(EventKind::Marker, format!("end:{step}:{}", if succeeded { "ok" } else { "failed" }));
    }

    pub fn finish(&self, title: &str) -> Recording {
        let events = self.events.lock().map(|e| e.clone()).unwrap_or_default();
        Recording { width: self.width, height: self.height, title: title.to_owned(), events }
    }
}

impl Recording {
    fn marker_index(&self, label: impl Fn(&str) -> bool) -> Option<usize> {
        self.events.iter().position(|e| e.kind == EventKind::Marker && label(&e.data))
    }

    /// Every step recorded, in the order it started.
    pub fn steps(&self) -> Vec<StepSpan> {
        let mut spans: Vec<StepSpan> = Vec::new();
        for e in self.events.iter().filter(|e| e.kind == EventKind::Marker) {
            if let Some(step) = e.data.strip_prefix("start:") {
                spans.push(StepSpan { step: step.to_owned(), start: e.at, end: None, succeeded: None });
            } else if let Some(rest) = e.data.strip_prefix("end:")
                && let Some((step, how)) = rest.rsplit_once(':')
                && let Some(span) = spans.iter_mut().rev().find(|s| s.step == step && s.end.is_none())
            {
                span.end = Some(e.at);
                span.succeeded = Some(how == "ok");
            }
        }
        spans
    }

    /// The screen as it was just before event `index`: its size, and what
    /// redraws it from a cleared one.
    fn screen_before(&self, index: usize) -> (u16, u16, String) {
        let mut screen = crate::transcript::Transcript::new(self.height, self.width);
        for e in &self.events[..index] {
            match e.kind {
                EventKind::Output => screen.feed(&e.data),
                EventKind::Resize => {
                    if let Some((cols, rows)) = size_of(&e.data) {
                        screen.resize(rows, cols);
                    }
                }
                EventKind::Marker => {}
            }
        }
        let (rows, cols) = screen.size();
        (cols, rows, screen.formatted())
    }

    /// From the start of step `first` to the end of step `last`. The cut
    /// opens on the screen as it was at that moment, at the size it had,
    /// not a blank one and not everything before it replayed.
    pub fn cut(&self, first: &str, last: &str) -> Result<Recording, String> {
        let start = self
            .marker_index(|m| m == format!("start:{first}"))
            .ok_or_else(|| format!("step `{first}` is not in this recording"))?;
        let end_of_last = self.marker_index(|m| {
            m.strip_prefix("end:").and_then(|r| r.rsplit_once(':')).is_some_and(|(s, _)| s == last)
        });
        let started_last = self
            .marker_index(|m| m == format!("start:{last}"))
            .ok_or_else(|| format!("step `{last}` is not in this recording"))?;
        if started_last < start {
            return Err(format!("step `{last}` comes before `{first}`"));
        }
        let end = end_of_last.filter(|&e| e >= started_last).unwrap_or(self.events.len() - 1);
        let t0 = self.events[start].at;
        let (width, height, screen) = self.screen_before(start);
        let mut events = vec![Event { at: 0.0, kind: EventKind::Output, data: screen }];
        events.extend(self.events[start..=end].iter().map(|e| Event { at: e.at - t0, ..e.clone() }));
        Ok(Recording { width, height, events, title: self.title.clone() })
    }

    /// Pauses longer than `max` seconds shortened to `max`.
    pub fn limit_idle(&self, max: f64) -> Recording {
        let mut shift = 0.0;
        let mut previous = 0.0;
        let events = self
            .events
            .iter()
            .map(|e| {
                let gap = e.at - previous;
                if gap > max {
                    shift += gap - max;
                }
                previous = e.at;
                Event { at: e.at - shift, ..e.clone() }
            })
            .collect();
        Recording { events, ..self.clone() }
    }

    /// With every recognisable secret in the output replaced by `*`, one for
    /// each character, wherever its characters fell. Secrets are found in
    /// the text as the screen shows it, escape sequences left out, so one
    /// drawn in pieces or in colour is found whole. Returns the kinds found,
    /// with counts, never the values.
    pub fn redacted(&self) -> (Recording, Vec<String>) {
        self.redacted_for(&[])
    }

    /// [`Recording::redacted`], with `names` masked too wherever one stands
    /// on its own ([`name_spans`]): the account's name, for an export that
    /// may be shared, since it is in every path under the profile.
    pub fn redacted_for(&self, names: &[String]) -> (Recording, Vec<String>) {
        // Each visible character: (event, byte offset in its data, length).
        let mut visible = String::new();
        let mut places: Vec<(usize, usize, usize)> = Vec::new();
        let mut state = Ansi::Normal;
        for (i, e) in self.events.iter().enumerate().filter(|(_, e)| e.kind == EventKind::Output) {
            for (offset, c) in e.data.char_indices() {
                if state.step(c) {
                    places.push((i, offset, c.len_utf8()));
                    visible.push(c);
                }
            }
        }
        let mut starts = Vec::with_capacity(places.len());
        let mut at = 0;
        for &(_, _, len) in &places {
            starts.push(at);
            at += len;
        }
        let mut masked: Vec<Vec<(usize, usize)>> = vec![Vec::new(); self.events.len()];
        let mut counts: Vec<(&'static str, usize)> = Vec::new();
        let names = names.iter().flat_map(|name| name_spans(&visible, name)).map(|r| (r, ACCOUNT_NAME));
        for (range, kind) in keyjutsu_agent::context::secret_spans(&visible).into_iter().chain(names) {
            match counts.iter_mut().find(|(k, _)| *k == kind) {
                Some((_, n)) => *n += 1,
                None => counts.push((kind, 1)),
            }
            // `starts` is in order: only the characters inside `range`.
            let first = starts.partition_point(|&s| s < range.start);
            for n in first..starts.len() {
                if starts[n] >= range.end {
                    break;
                }
                let (event, offset, len) = places[n];
                let c = visible[starts[n]..].chars().next().unwrap_or(' ');
                if !c.is_control() {
                    masked[event].push((offset, len));
                }
            }
        }
        let events = self
            .events
            .iter()
            .zip(masked)
            .map(|(e, mut mask)| {
                if mask.is_empty() {
                    return e.clone();
                }
                mask.sort_unstable();
                // A character inside both a secret and a name is masked once.
                mask.dedup();
                let mut data = String::with_capacity(e.data.len());
                let mut from = 0;
                for (offset, len) in mask {
                    data.push_str(&e.data[from..offset]);
                    data.push('*');
                    from = offset + len;
                }
                data.push_str(&e.data[from..]);
                Event { data, ..e.clone() }
            })
            .collect();
        let found = counts.into_iter().map(|(k, n)| format!("{k} ×{n}")).collect();
        (Recording { events, ..self.clone() }, found)
    }

    /// What step `step` printed, as the terminal showed it: the command's
    /// line and its output, read from a screen the recording's size.
    pub fn step_text(&self, step: &str, max_chars: usize) -> Option<String> {
        let start = self.marker_index(|m| m == format!("start:{step}"))?;
        let end = self
            .events
            .iter()
            .enumerate()
            .skip(start)
            .find(|(_, e)| {
                e.kind == EventKind::Marker
                    && e.data
                        .strip_prefix("end:")
                        .and_then(|r| r.rsplit_once(':'))
                        .is_some_and(|(s, _)| s == step)
            })
            .map_or(self.events.len(), |(i, _)| i);
        let mut screen = crate::transcript::Transcript::new(self.height, self.width);
        let play = |screen: &mut crate::transcript::Transcript, e: &Event| match e.kind {
            EventKind::Output => screen.feed(&e.data),
            EventKind::Resize => {
                if let Some((cols, rows)) = size_of(&e.data) {
                    screen.resize(rows, cols);
                }
            }
            EventKind::Marker => {}
        };
        for e in &self.events[..start] {
            play(&mut screen, e);
        }
        let mark = screen.mark();
        for e in &self.events[start..end] {
            play(&mut screen, e);
        }
        Some(screen.since(mark, max_chars))
    }

    /// As asciicast v2.
    pub fn to_cast(&self) -> String {
        let header = json!({"version": 2, "width": self.width, "height": self.height, "title": self.title});
        let mut out = header.to_string();
        for e in &self.events {
            let code = match e.kind {
                EventKind::Output => "o",
                EventKind::Marker => "m",
                EventKind::Resize => "r",
            };
            // Three decimal places, as asciinema writes them.
            let at = (e.at * 1000.0).round() / 1000.0;
            out.push('\n');
            out.push_str(&json!([at, code, e.data]).to_string());
        }
        out.push('\n');
        out
    }

    /// Read asciicast v2. Events other than output and markers are skipped.
    pub fn from_cast(text: &str) -> Result<Recording, String> {
        let mut lines = text.lines().filter(|l| !l.trim().is_empty());
        let header: Value = serde_json::from_str(lines.next().ok_or("the recording is empty")?)
            .map_err(|e| format!("the recording's header is not JSON: {e}"))?;
        if header["version"] != 2 {
            return Err("only asciicast version 2 is read".into());
        }
        let size = |k: &str| header[k].as_u64().and_then(|n| u16::try_from(n).ok()).unwrap_or(80);
        let mut events = Vec::new();
        for (n, line) in lines.enumerate() {
            let v: Value = serde_json::from_str(line).map_err(|e| format!("event {}: {e}", n + 1))?;
            let (Some(at), Some(code), Some(data)) = (v[0].as_f64(), v[1].as_str(), v[2].as_str()) else {
                return Err(format!("event {} is not [time, code, data]", n + 1));
            };
            let kind = match code {
                "o" => EventKind::Output,
                "m" => EventKind::Marker,
                "r" => EventKind::Resize,
                _ => continue,
            };
            events.push(Event { at, kind, data: data.to_owned() });
        }
        Ok(Recording {
            width: size("width"),
            height: size("height"),
            title: header["title"].as_str().unwrap_or_default().to_owned(),
            events,
        })
    }
}

/// How an account's name is listed among what an export took out.
const ACCOUNT_NAME: &str = "account name";

/// What names this person on this machine, to keep out of an export: the
/// account's name, and its profile folder's, which can differ (a renamed
/// account keeps its folder). A one-letter name is left alone: masking every
/// "a" standing on its own would hide more than it protects.
pub fn account_names() -> Vec<String> {
    let folder = std::env::var("USERPROFILE")
        .ok()
        .and_then(|p| std::path::Path::new(&p).file_name().map(|n| n.to_string_lossy().into_owned()));
    let mut names: Vec<String> = Vec::new();
    for name in [std::env::var("USERNAME").ok(), folder].into_iter().flatten() {
        let name = name.trim().to_owned();
        if name.chars().count() >= 2 && !names.iter().any(|n| n.to_lowercase() == name.to_lowercase()) {
            names.push(name);
        }
    }
    names
}

/// Where `name` stands on its own in `text`, in any case: `nrtat` in
/// `C:\Users\nrtat>` or `nrtat@host`, not inside a longer word.
pub fn name_spans(text: &str, name: &str) -> Vec<std::ops::Range<usize>> {
    let wanted: Vec<char> = name.chars().collect();
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    if wanted.is_empty() || wanted.len() > chars.len() {
        return Vec::new();
    }
    let same = |a: char, b: char| a == b || a.to_lowercase().eq(b.to_lowercase());
    let word = |c: Option<&(usize, char)>| c.is_some_and(|&(_, c)| c.is_alphanumeric());
    let mut spans = Vec::new();
    let mut i = 0;
    while i + wanted.len() <= chars.len() {
        let end = i + wanted.len();
        if chars[i..end].iter().zip(&wanted).all(|(&(_, c), &w)| same(c, w))
            && !word(i.checked_sub(1).and_then(|p| chars.get(p)))
            && !word(chars.get(end))
        {
            spans.push(chars[i].0..chars.get(end).map_or(text.len(), |&(b, _)| b));
            i = end;
        } else {
            i += 1;
        }
    }
    spans
}

/// `text` with each of `names` standing on its own masked, one `*` for each
/// character, and how many were.
pub fn mask_names(text: &str, names: &[String]) -> (String, usize) {
    let mut spans: Vec<std::ops::Range<usize>> = names.iter().flat_map(|n| name_spans(text, n)).collect();
    spans.sort_by_key(|r| r.start);
    let mut out = String::with_capacity(text.len());
    let mut from = 0;
    let mut count = 0;
    for r in spans {
        if r.start < from {
            continue;
        }
        out.push_str(&text[from..r.start]);
        out.extend(text[r.clone()].chars().map(|_| '*'));
        from = r.end;
        count += 1;
    }
    out.push_str(&text[from..]);
    (out, count)
}

/// A step as the guide describes it: from the plan, and what it printed.
#[derive(Debug, Clone, PartialEq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "recording/")]
pub struct GuideStep {
    pub step: String,
    pub title: String,
    pub objective: String,
    #[ts(optional)]
    pub reason: Option<String>,
    pub commands: Vec<String>,
    /// What it printed, as the terminal showed it, redacted.
    pub printed: String,
    #[ts(optional)]
    pub succeeded: Option<bool>,
}

/// A run's recording, cut and made ready to export: redacted, with long
/// pauses shortened, as asciicast too, and each step for the guide. The
/// account's name is masked in all of it, the guide's text from the plan too.
#[derive(Debug, Clone, PartialEq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "recording/")]
pub struct Export {
    pub title: String,
    pub recording: Recording,
    pub cast: String,
    /// Kinds of secret taken out, and the account's name, with counts, never
    /// the values.
    pub redactions: Vec<String>,
    pub steps: Vec<GuideStep>,
}

/// Run `record`'s recording `whole`, from step `first` to step `last` (the
/// whole run if neither is given; the run's first or last step for the one
/// left out), ready to export, with `names` ([`account_names`]) masked.
pub fn prepare_export(
    record: &crate::history::SessionRecord,
    whole: &Recording,
    first: Option<&str>,
    last: Option<&str>,
    names: &[String],
) -> Result<Export, String> {
    let spans = whole.steps();
    let cut = match (first, last) {
        (None, None) => whole.clone(),
        (f, l) => {
            let f = f.map(str::to_owned).or_else(|| spans.first().map(|s| s.step.clone()));
            let l = l.map(str::to_owned).or_else(|| spans.last().map(|s| s.step.clone()));
            match (f, l) {
                (Some(f), Some(l)) => whole.cut(&f, &l)?,
                _ => return Err("the recording has no steps".into()),
            }
        }
    };
    let (clean, mut redactions) = cut.redacted_for(names);
    let mut recording = clean.limit_idle(IDLE_LIMIT);
    let snapshot = record.snapshot()?;
    let plan = snapshot.plan();
    // The guide's words come from the plan, whose paths name the account too.
    let mut in_text = 0;
    let mut mask = |text: &str| {
        let (masked, n) = mask_names(text, names);
        in_text += n;
        masked
    };
    recording.title = mask(&recording.title);
    let title = mask(&record.task);
    let steps = recording
        .steps()
        .into_iter()
        .map(|span| {
            let step = plan.step(&span.step);
            GuideStep {
                title: mask(&step.map_or_else(|| span.step.clone(), |s| s.title.clone())),
                objective: mask(&step.map(|s| s.objective.clone()).unwrap_or_default()),
                reason: step.and_then(|s| s.reason.as_deref()).map(&mut mask),
                commands: step
                    .map(|s| s.commands.iter().map(|c| mask(&c.text)).collect())
                    .unwrap_or_default(),
                printed: recording.step_text(&span.step, 4000).unwrap_or_default(),
                succeeded: span.succeeded,
                step: span.step,
            }
        })
        .collect();
    if in_text > 0 {
        let prefix = format!("{ACCOUNT_NAME} ×");
        match redactions.iter_mut().find(|r| r.starts_with(&prefix)) {
            Some(r) => {
                let before: usize = r[prefix.len()..].parse().unwrap_or(0);
                *r = format!("{prefix}{}", before + in_text);
            }
            None => redactions.push(format!("{prefix}{in_text}")),
        }
    }
    Ok(Export { title, cast: recording.to_cast(), recording, redactions, steps })
}

/// Just enough of a terminal's escape-sequence grammar to tell what is drawn
/// from what only moves the cursor or sets colours.
#[derive(Clone, Copy)]
enum Ansi {
    Normal,
    Escape,
    Csi,
    Osc,
    OscEscape,
}

impl Ansi {
    /// Advance by `c`; true if `c` is drawn.
    fn step(&mut self, c: char) -> bool {
        let (next, drawn) = match (*self, c) {
            (Ansi::Normal, '\u{1b}') => (Ansi::Escape, false),
            (Ansi::Normal, _) => (Ansi::Normal, true),
            (Ansi::Escape, '[') => (Ansi::Csi, false),
            (Ansi::Escape, ']') => (Ansi::Osc, false),
            (Ansi::Escape, _) => (Ansi::Normal, false),
            (Ansi::Csi, '\u{40}'..='\u{7e}') => (Ansi::Normal, false),
            (Ansi::Csi, _) => (Ansi::Csi, false),
            (Ansi::Osc, '\u{7}') => (Ansi::Normal, false),
            (Ansi::Osc, '\u{1b}') => (Ansi::OscEscape, false),
            (Ansi::Osc, _) => (Ansi::Osc, false),
            (Ansi::OscEscape, '\\') => (Ansi::Normal, false),
            (Ansi::OscEscape, _) => (Ansi::Osc, false),
        };
        *self = next;
        drawn
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(at: f64, kind: EventKind, data: &str) -> Event {
        Event { at, kind, data: data.into() }
    }

    fn sample() -> Recording {
        use EventKind::{Marker, Output};
        Recording {
            width: 80,
            height: 24,
            title: "Two steps".into(),
            events: vec![
                ev(0.0, Output, "PS> "),
                ev(1.0, Marker, "start:look"),
                ev(1.1, Output, "Get-Date\r\n"),
                ev(1.5, Output, "Monday\r\nPS> "),
                ev(1.6, Marker, "end:look:ok"),
                ev(30.0, Marker, "start:say"),
                ev(30.2, Output, "Write-Output hi\r\nhi\r\nPS> "),
                ev(30.3, Marker, "end:say:failed"),
            ],
        }
    }

    #[test]
    fn steps_are_found_with_how_they_ended() {
        let s = sample().steps();
        assert_eq!(s.len(), 2);
        assert_eq!(
            (s[0].step.as_str(), s[0].start, s[0].end, s[0].succeeded),
            ("look", 1.0, Some(1.6), Some(true))
        );
        assert_eq!((s[1].step.as_str(), s[1].succeeded), ("say", Some(false)));
    }

    /// A cut opens on the screen as it was: what came before is replayed at
    /// once, and the step's own events keep their pace.
    #[test]
    fn a_cut_opens_on_the_screen_as_it_was_then_keeps_the_steps_pace() {
        let c = sample().cut("say", "say").unwrap();
        let opening = &c.events[0];
        assert_eq!((opening.at, opening.kind), (0.0, EventKind::Output));
        let mut shown = crate::transcript::Transcript::new(c.height, c.width);
        shown.feed(&opening.data);
        let text = shown.since(0, 1000);
        assert!(text.contains("Get-Date") && text.contains("Monday"), "{text}");
        assert_eq!(c.events[1], ev(0.0, EventKind::Marker, "start:say"));
        assert!((c.events[2].at - 0.2).abs() < 1e-9);
        assert_eq!(c.events.last().unwrap().data, "end:say:failed");
        let both = sample().cut("look", "say").unwrap();
        assert_eq!(both.steps().len(), 2);
        assert!(sample().cut("say", "look").is_err());
        assert!(sample().cut("nowhere", "say").is_err());
    }

    #[test]
    fn long_pauses_are_shortened() {
        let r = sample().limit_idle(IDLE_LIMIT);
        let start_say = r.events.iter().find(|e| e.data == "start:say").unwrap();
        assert!((start_say.at - (1.6 + IDLE_LIMIT)).abs() < 1e-9, "{}", start_say.at);
        assert!(r.events.windows(2).all(|w| w[1].at - w[0].at <= IDLE_LIMIT + 1e-9));
    }

    /// Typed a character at a time, in colour: found in what the screen
    /// shows, masked where each character fell, the rest left as it was.
    #[test]
    fn a_secret_drawn_in_pieces_is_masked_where_it_fell() {
        use EventKind::Output;
        let token = "ghp_0123456789abcdefghijABCDEFGHIJ012345";
        let mut events = vec![ev(0.0, Output, "echo \u{1b}[93m")];
        for (n, c) in token.chars().enumerate() {
            events.push(ev(0.1 * n as f64, Output, &c.to_string()));
        }
        events.push(ev(9.0, Output, "\u{1b}[0m\r\ndone"));
        let r = Recording { width: 80, height: 24, title: String::new(), events };
        let (clean, found) = r.redacted();
        assert_eq!(found, ["GitHub token ×1"]);
        let all: String = clean.events.iter().map(|e| e.data.as_str()).collect();
        assert!(!all.contains("ghp_"), "{all}");
        assert_eq!(all, format!("echo \u{1b}[93m{}\u{1b}[0m\r\ndone", "*".repeat(token.len())));
        assert_eq!(clean.events.len(), r.events.len(), "timing kept");
        assert!(sample().redacted().1.is_empty());
    }

    /// The account's name, drawn in pieces and in colour, is masked where it
    /// stands on its own, in any case, and nowhere inside a longer word.
    #[test]
    fn the_account_name_is_masked_where_it_stands_alone() {
        use EventKind::Output;
        let events = vec![
            ev(0.0, Output, "PS C:\\Users\\nr"),
            ev(0.1, Output, "\u{1b}[93mtat\u{1b}[0m> dir\r\n"),
            ev(0.2, Output, "NRTAT@pc  nrtatx  xnrtat  nrtat\r\n"),
        ];
        let r = Recording { width: 80, height: 24, title: String::new(), events };
        let (clean, found) = r.redacted_for(&["nrtat".to_owned()]);
        assert_eq!(found, ["account name ×3"]);
        let all: String = clean.events.iter().map(|e| e.data.as_str()).collect();
        assert_eq!(all, "PS C:\\Users\\**\u{1b}[93m***\u{1b}[0m> dir\r\n*****@pc  nrtatx  xnrtat  *****\r\n");
        assert_eq!(r.redacted().1, Vec::<String>::new(), "without names, only secrets");
    }

    #[test]
    fn text_from_the_plan_is_masked_the_same_way() {
        let names = ["nrtat".to_owned(), "Nigel Tatum".to_owned()];
        let (text, n) = mask_names(r"$d = 'C:\Users\nrtat\OneDrive - Nigel Tatum\Desktop'", &names);
        assert_eq!(text, r"$d = 'C:\Users\*****\OneDrive - ***********\Desktop'");
        assert_eq!(n, 2);
        assert_eq!(mask_names("nrtatx", &names), ("nrtatx".to_owned(), 0));
        assert!(name_spans("abc", "").is_empty());
    }

    #[test]
    fn a_steps_text_is_what_it_printed() {
        let text = sample().step_text("say", 1000).unwrap();
        assert!(text.contains("Write-Output hi") && text.contains("hi"), "{text}");
        assert!(!text.contains("Monday"), "{text}");
        assert!(sample().step_text("nowhere", 1000).is_none());
    }

    /// Drawn for one size and replayed at another, a terminal's output lands
    /// in the wrong places: a recording follows every change of size, and a
    /// cut opens at the size the terminal had then.
    #[test]
    fn a_recording_follows_the_terminals_size() {
        let rec = Recorder::new(20, 5);
        rec.output("PS> ");
        rec.resize(60, 5);
        rec.step_started("wide");
        rec.output(
            "Write-Output 'a line longer than twenty columns'\r\na line longer than twenty columns\r\n",
        );
        rec.step_finished("wide", true);
        let r = rec.finish("t");
        let text = r.step_text("wide", 1000).unwrap();
        assert!(text.contains("a line longer than twenty columns\n") || text.ends_with("columns"), "{text}");
        assert!(!text.contains("colum\nns"), "not wrapped at twenty: {text}");
        let c = r.cut("wide", "wide").unwrap();
        assert_eq!((c.width, c.height), (60, 5), "the cut opens at the size then");
        let cast = r.to_cast();
        assert!(cast.contains("\"r\",\"60x5\""), "{cast}");
        // Times are kept to the millisecond, as asciinema keeps them.
        let back = Recording::from_cast(&cast).unwrap();
        let what = |r: &Recording| r.events.iter().map(|e| (e.kind, e.data.clone())).collect::<Vec<_>>();
        assert_eq!(what(&back), what(&r));
        assert_eq!(size_of("120x30"), Some((120, 30)));
        assert_eq!(size_of("nonsense"), None);
    }

    #[test]
    fn asciicast_is_written_and_read_back() {
        let r = sample();
        let cast = r.to_cast();
        assert!(cast.starts_with("{\"") && cast.contains("\"version\":2"), "{cast}");
        assert!(cast.contains("[1.0,\"m\",\"start:look\"]"), "{cast}");
        assert_eq!(Recording::from_cast(&cast).unwrap(), r);
        assert!(Recording::from_cast("{\"version\":1}").is_err());
    }

    #[test]
    fn the_recorder_keeps_output_and_markers_in_order() {
        let rec = Recorder::new(100, 30);
        rec.output("PS> ");
        rec.step_started("look");
        rec.output("Get-Date\r\n");
        rec.step_finished("look", true);
        let r = rec.finish("Look");
        assert_eq!((r.width, r.height, r.title.as_str()), (100, 30, "Look"));
        let kinds: Vec<&str> = r.events.iter().map(|e| e.data.as_str()).collect();
        assert_eq!(kinds, ["PS> ", "start:look", "Get-Date\r\n", "end:look:ok"]);
        assert!(r.events.windows(2).all(|w| w[0].at <= w[1].at));
    }
}
