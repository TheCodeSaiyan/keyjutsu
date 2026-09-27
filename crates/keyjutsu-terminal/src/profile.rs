//! Matching the user's own terminal.
//!
//! When Performance Mode is armed the window should look like the terminal the
//! user normally works in. On Windows that is usually Windows Terminal, whose
//! settings name the default profile's shell, font, colour scheme, cursor and
//! padding. This module reads them; it never writes them.
//!
//! It also inspects the PowerShell profile for tools known to redraw the
//! prompt or the input line (oh-my-posh, Starship, PSReadLine options), which
//! is what decides whether the compatibility profile is offered. Only the
//! findings are kept, never the profile's contents.

use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::Value;

use crate::shell::ShellKind;

#[derive(Debug, Clone, PartialEq, Serialize, ts_rs::TS)]
#[ts(export)]
pub struct ColorScheme {
    pub name: String,
    pub background: String,
    pub foreground: String,
    pub cursor: String,
    pub selection_background: String,
    /// black, red, green, yellow, blue, purple, cyan, white, then the eight
    /// bright variants, in Windows Terminal's order.
    pub ansi: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum CursorShape {
    Bar,
    Underline,
    Block,
}

#[derive(Debug, Clone, PartialEq, Serialize, ts_rs::TS)]
#[ts(export)]
pub struct TerminalProfile {
    /// Where the settings came from: a settings file path, or `built-in` when
    /// no Windows Terminal settings were found.
    pub source: String,
    pub name: String,
    pub shell: Option<ShellKind>,
    pub commandline: Option<String>,
    pub font_face: String,
    pub font_size: f32,
    pub color_scheme: ColorScheme,
    pub cursor_shape: CursorShape,
    /// Left, top, right, bottom, in pixels.
    pub padding: [u16; 4],
    pub starting_directory: Option<String>,
}

impl TerminalProfile {
    /// Windows Terminal's own defaults, used when nothing is configured.
    pub fn built_in() -> Self {
        Self {
            source: "built-in".into(),
            name: "Windows PowerShell".into(),
            shell: None,
            commandline: None,
            font_face: "Cascadia Mono".into(),
            font_size: 12.0,
            color_scheme: built_in_scheme("Campbell").unwrap_or_else(campbell),
            cursor_shape: CursorShape::Bar,
            padding: [8, 8, 8, 8],
            starting_directory: None,
        }
    }
}

/// Settings file locations, in the order Windows Terminal itself would be
/// found: the Store release, Preview, then an unpackaged install.
pub fn windows_terminal_settings_candidates() -> Vec<PathBuf> {
    let Some(local) = std::env::var_os("LOCALAPPDATA").map(PathBuf::from) else {
        return Vec::new();
    };
    vec![
        local.join(r"Packages\Microsoft.WindowsTerminal_8wekyb3d8bbwe\LocalState\settings.json"),
        local.join(r"Packages\Microsoft.WindowsTerminalPreview_8wekyb3d8bbwe\LocalState\settings.json"),
        local.join(r"Microsoft\Windows Terminal\settings.json"),
    ]
}

/// The user's default Windows Terminal profile, or the built-in defaults.
pub fn detect_terminal_profile() -> TerminalProfile {
    windows_terminal_settings_candidates()
        .into_iter()
        .find_map(|path| {
            let text = std::fs::read_to_string(&path).ok()?;
            parse_windows_terminal_settings(&text, &path.display().to_string())
        })
        .unwrap_or_else(TerminalProfile::built_in)
}

/// Resolve the default profile from Windows Terminal settings text.
pub fn parse_windows_terminal_settings(text: &str, source: &str) -> Option<TerminalProfile> {
    let settings: Value = serde_json::from_str(&strip_jsonc(text)).ok()?;
    let default_guid = settings.get("defaultProfile").and_then(Value::as_str);
    let (defaults, list) = match settings.get("profiles") {
        Some(Value::Array(list)) => (None, list.clone()),
        Some(Value::Object(p)) => {
            (p.get("defaults").cloned(), p.get("list").and_then(Value::as_array).cloned().unwrap_or_default())
        }
        _ => (None, Vec::new()),
    };
    let chosen = list
        .iter()
        .find(|p| {
            default_guid.is_some_and(|g| {
                p.get("guid").and_then(Value::as_str).is_some_and(|pg| pg.eq_ignore_ascii_case(g))
            })
        })
        .or_else(|| list.iter().find(|p| !p.get("hidden").and_then(Value::as_bool).unwrap_or(false)))?;

    // A profile's own value wins over `profiles.defaults`.
    let get = |key: &str| -> Option<&Value> {
        chosen.get(key).or_else(|| defaults.as_ref().and_then(|d| d.get(key)))
    };
    let font = get("font");
    let font_face = font
        .and_then(|f| f.get("face"))
        .or_else(|| get("fontFace"))
        .and_then(Value::as_str)
        .unwrap_or("Cascadia Mono")
        .to_owned();
    let font_size =
        font.and_then(|f| f.get("size")).or_else(|| get("fontSize")).and_then(Value::as_f64).unwrap_or(12.0)
            as f32;

    let name = chosen.get("name").and_then(Value::as_str).unwrap_or("Default").to_owned();
    let commandline = chosen.get("commandline").and_then(Value::as_str).map(str::to_owned);
    let source_hint = chosen.get("source").and_then(Value::as_str);
    let shell = infer_shell(commandline.as_deref(), source_hint, &name);

    let scheme_name = get("colorScheme").and_then(Value::as_str).unwrap_or(match shell {
        Some(ShellKind::WindowsPowershell) => "Campbell Powershell",
        _ => "Campbell",
    });
    let color_scheme = settings
        .get("schemes")
        .and_then(Value::as_array)
        .and_then(|schemes| {
            schemes.iter().find(|s| s.get("name").and_then(Value::as_str) == Some(scheme_name))
        })
        .and_then(scheme_from_json)
        .or_else(|| built_in_scheme(scheme_name))
        .unwrap_or_else(campbell);

    let cursor_shape = match get("cursorShape").and_then(Value::as_str) {
        Some("underscore" | "doubleUnderscore") => CursorShape::Underline,
        Some("filledBox" | "emptyBox" | "vintage") => CursorShape::Block,
        _ => CursorShape::Bar,
    };
    let padding = get("padding").and_then(Value::as_str).and_then(parse_padding).unwrap_or([8, 8, 8, 8]);
    let starting_directory = get("startingDirectory").and_then(Value::as_str).map(str::to_owned);

    Some(TerminalProfile {
        source: source.to_owned(),
        name,
        shell,
        commandline,
        font_face,
        font_size,
        color_scheme,
        cursor_shape,
        padding,
        starting_directory,
    })
}

fn infer_shell(commandline: Option<&str>, source: Option<&str>, name: &str) -> Option<ShellKind> {
    let haystack =
        format!("{} {} {}", commandline.unwrap_or(""), source.unwrap_or(""), name).to_ascii_lowercase();
    if haystack.contains("pwsh") || haystack.contains("powershellcore") || haystack.contains("powershell 7") {
        Some(ShellKind::Pwsh)
    } else if haystack.contains("powershell") {
        Some(ShellKind::WindowsPowershell)
    } else if haystack.contains("cmd") || haystack.contains("command prompt") {
        Some(ShellKind::Cmd)
    } else {
        None
    }
}

/// `"8"`, `"8, 4"` or `"8, 4, 8, 4"`, as Windows Terminal accepts them.
fn parse_padding(text: &str) -> Option<[u16; 4]> {
    let v: Vec<u16> = text.split(',').map(|s| s.trim().parse().ok()).collect::<Option<_>>()?;
    match v.as_slice() {
        [a] => Some([*a; 4]),
        [h, v] => Some([*h, *v, *h, *v]),
        [l, t, r, b] => Some([*l, *t, *r, *b]),
        _ => None,
    }
}

const ANSI_KEYS: [&str; 16] = [
    "black",
    "red",
    "green",
    "yellow",
    "blue",
    "purple",
    "cyan",
    "white",
    "brightBlack",
    "brightRed",
    "brightGreen",
    "brightYellow",
    "brightBlue",
    "brightPurple",
    "brightCyan",
    "brightWhite",
];

fn scheme_from_json(s: &Value) -> Option<ColorScheme> {
    let field = |k: &str| s.get(k).and_then(Value::as_str).map(str::to_owned);
    let foreground = field("foreground")?;
    Some(ColorScheme {
        name: field("name")?,
        background: field("background")?,
        cursor: field("cursorColor").unwrap_or_else(|| foreground.clone()),
        selection_background: field("selectionBackground").unwrap_or_else(|| "#FFFFFF".into()),
        ansi: ANSI_KEYS.iter().map(|k| field(k)).collect::<Option<_>>()?,
        foreground,
    })
}

fn campbell() -> ColorScheme {
    scheme_with("Campbell", "#0C0C0C")
}

fn scheme_with(name: &str, background: &str) -> ColorScheme {
    ColorScheme {
        name: name.into(),
        background: background.into(),
        foreground: "#CCCCCC".into(),
        cursor: "#FFFFFF".into(),
        selection_background: "#FFFFFF".into(),
        ansi: [
            "#0C0C0C", "#C50F1F", "#13A10E", "#C19C00", "#0037DA", "#881798", "#3A96DD", "#CCCCCC",
            "#767676", "#E74856", "#16C60C", "#F9F1A5", "#3B78FF", "#B4009E", "#61D6D6", "#F2F2F2",
        ]
        .map(String::from)
        .to_vec(),
    }
}

/// Windows Terminal ships these without listing them in settings.json. Only
/// the two defaults are carried; any other built-in name falls back to
/// Campbell rather than being guessed at.
fn built_in_scheme(name: &str) -> Option<ColorScheme> {
    match name {
        "Campbell" => Some(scheme_with("Campbell", "#0C0C0C")),
        "Campbell Powershell" => Some(scheme_with("Campbell Powershell", "#012456")),
        _ => None,
    }
}

/// Remove `//` and `/* */` comments and trailing commas, which Windows
/// Terminal accepts and `serde_json` does not. String contents are untouched.
pub fn strip_jsonc(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    let mut in_string = false;
    while i < chars.len() {
        let c = chars[i];
        if in_string {
            out.push(c);
            if c == '\\' && i + 1 < chars.len() {
                out.push(chars[i + 1]);
                i += 2;
                continue;
            }
            if c == '"' {
                in_string = false;
            }
            i += 1;
            continue;
        }
        match (c, chars.get(i + 1)) {
            ('"', _) => {
                in_string = true;
                out.push(c);
                i += 1;
            }
            ('/', Some('/')) => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
            }
            ('/', Some('*')) => {
                i += 2;
                while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '/') {
                    i += 1;
                }
                i += 2;
            }
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }
    // Trailing commas are removed only once the comments have gone, so a
    // comment between a comma and the closing bracket cannot hide it.
    strip_trailing_commas(&out)
}

fn strip_trailing_commas(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut in_string = false;
    let mut escaped = false;
    for (i, &c) in chars.iter().enumerate() {
        if in_string {
            match (escaped, c) {
                (true, _) => escaped = false,
                (false, '\\') => escaped = true,
                (false, '"') => in_string = false,
                _ => {}
            }
        } else if c == '"' {
            in_string = true;
        } else if c == ',' && matches!(chars[i + 1..].iter().find(|c| !c.is_whitespace()), Some('}' | ']')) {
            continue;
        }
        out.push(c);
    }
    out
}

/// What the PowerShell profile does that could disturb staged input.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, ts_rs::TS)]
#[ts(export)]
pub struct PowerShellProfileReport {
    /// Profile scripts that exist for the current user and host.
    pub scripts: Vec<String>,
    pub uses_oh_my_posh: bool,
    pub uses_starship: bool,
    /// The profile changes PSReadLine behaviour, for example predictions or
    /// key handlers that insert text.
    pub configures_psreadline: bool,
}

impl PowerShellProfileReport {
    /// Anything that redraws the prompt or reacts to individual keystrokes is
    /// a reason to test the profile before a performance.
    pub fn may_interfere(&self) -> bool {
        self.uses_oh_my_posh || self.uses_starship || self.configures_psreadline
    }
}

/// Inspect the profile scripts `program` would load. Asks the shell for its
/// `$PROFILE` paths without loading them.
pub fn inspect_powershell_profile(program: &Path) -> PowerShellProfileReport {
    let paths = crate::shell::command(program)
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "$PROFILE.AllUsersAllHosts; $PROFILE.AllUsersCurrentHost; $PROFILE.CurrentUserAllHosts; $PROFILE.CurrentUserCurrentHost",
        ])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    let mut report = PowerShellProfileReport::default();
    for line in paths.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if let Ok(content) = std::fs::read_to_string(line) {
            report.scripts.push(line.to_owned());
            report.absorb(&content);
        }
    }
    report
}

impl PowerShellProfileReport {
    fn absorb(&mut self, content: &str) {
        let lower = content.to_ascii_lowercase();
        self.uses_oh_my_posh |= lower.contains("oh-my-posh");
        self.uses_starship |= lower.contains("starship");
        self.configures_psreadline |=
            lower.contains("set-psreadlineoption") || lower.contains("set-psreadlinekeyhandler");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SETTINGS: &str = include_str!("../../../tests/fixtures/windows-terminal/settings.json");

    #[test]
    fn strips_comments_and_trailing_commas_but_not_string_contents() {
        let text = r#"{ // comment
            "a": "http://x // not a comment", /* block */
            "b": [1, 2,],
        }"#;
        let v: Value = serde_json::from_str(&strip_jsonc(text)).unwrap();
        assert_eq!(v["a"], "http://x // not a comment");
        assert_eq!(v["b"], serde_json::json!([1, 2]));
    }

    #[test]
    fn resolves_the_default_profile_with_defaults_and_custom_scheme() {
        let p = parse_windows_terminal_settings(SETTINGS, "fixture").unwrap();
        assert_eq!(p.name, "PowerShell");
        assert_eq!(p.shell, Some(ShellKind::Pwsh));
        assert_eq!(p.font_face, "JetBrains Mono");
        assert_eq!(p.font_size, 11.0);
        assert_eq!(p.color_scheme.name, "Dusk");
        assert_eq!(p.color_scheme.background, "#1B1D23");
        assert_eq!(p.color_scheme.ansi.len(), 16);
        assert_eq!(p.cursor_shape, CursorShape::Block);
        assert_eq!(p.padding, [6, 4, 6, 4]);
    }

    #[test]
    fn falls_back_to_the_first_visible_profile_and_built_in_scheme() {
        let text = r#"{ "profiles": { "list": [
            { "name": "Hidden", "hidden": true },
            { "name": "Windows PowerShell", "commandline": "powershell.exe" }
        ] } }"#;
        let p = parse_windows_terminal_settings(text, "fixture").unwrap();
        assert_eq!(p.name, "Windows PowerShell");
        assert_eq!(p.shell, Some(ShellKind::WindowsPowershell));
        assert_eq!(p.color_scheme.name, "Campbell Powershell");
        assert_eq!(p.color_scheme.background, "#012456");
        assert_eq!(p.font_face, "Cascadia Mono");
    }

    #[test]
    fn padding_accepts_one_two_or_four_values() {
        assert_eq!(parse_padding("8"), Some([8, 8, 8, 8]));
        assert_eq!(parse_padding("8, 4"), Some([8, 4, 8, 4]));
        assert_eq!(parse_padding("1,2,3,4"), Some([1, 2, 3, 4]));
        assert_eq!(parse_padding("1,2,3"), None);
    }

    #[test]
    fn profile_findings_flag_prompt_and_input_tools() {
        let mut r = PowerShellProfileReport::default();
        r.absorb("oh-my-posh init pwsh --config ~/theme.json | Invoke-Expression");
        assert!(r.uses_oh_my_posh && r.may_interfere());
        let mut r = PowerShellProfileReport::default();
        r.absorb("Set-Alias ll Get-ChildItem");
        assert!(!r.may_interfere());
    }
}
