//! `keyjutsu setup`: what the installer offers and the uninstaller undoes.
//!
//! - `path`: put the folder holding `keyjutsu.exe` on this account's PATH.
//! - `explorer`: "Open KeyJutsu here" on folders and folder backgrounds.
//!
//! Both change only this Windows account (HKCU), both are idempotent, and
//! `remove` takes out exactly what `add` put in. PATH is read and written
//! without expanding the `%VARIABLES%` in it and keeps its registry type, so
//! a PATH that uses them is not flattened.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

fn norm(entry: &str) -> String {
    entry.trim().trim_end_matches(['\\', '/']).to_ascii_lowercase()
}

/// `path` with `dir` added at the end, unless it is already there.
pub fn path_with(path: &str, dir: &str) -> String {
    if path.split(';').any(|e| norm(e) == norm(dir)) {
        return path.to_owned();
    }
    let base = path.trim_end_matches(';');
    if base.is_empty() { dir.to_owned() } else { format!("{base};{dir}") }
}

/// `path` without `dir`, however it was written.
pub fn path_without(path: &str, dir: &str) -> String {
    path.split(';').filter(|e| !e.is_empty() && norm(e) != norm(dir)).collect::<Vec<_>>().join(";")
}

fn powershell(script: &str) -> Result<String, String> {
    let mut c = std::process::Command::new("powershell.exe");
    c.args(["-NoLogo", "-NoProfile", "-NonInteractive", "-EncodedCommand"]);
    c.arg(keyjutsu_core::terminal::shell::encode_powershell_command(script));
    let done =
        keyjutsu_core::validation::process::run(c, "", Duration::from_secs(60)).map_err(|e| e.to_string())?;
    if done.success {
        Ok(done.stdout.trim_end_matches(['\r', '\n']).to_owned())
    } else {
        Err(done.stderr.lines().find(|l| !l.trim().is_empty()).unwrap_or("PowerShell failed").to_owned())
    }
}

fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// This account's PATH as stored, and its registry type.
fn read_user_path() -> Result<(String, String), String> {
    let out = powershell(
        "$k = Get-Item -LiteralPath 'HKCU:\\Environment'\n\
         $kind = if ($k.GetValueNames() -contains 'Path') { $k.GetValueKind('Path').ToString() } else { 'ExpandString' }\n\
         $kind\n\
         $k.GetValue('Path', '', 'DoNotExpandEnvironmentNames')",
    )?;
    let mut lines = out.splitn(2, '\n');
    let kind = lines.next().unwrap_or("ExpandString").trim().to_owned();
    let value = lines.next().unwrap_or("").trim_end_matches(['\r', '\n']).to_owned();
    Ok((kind, value))
}

fn write_user_path(kind: &str, value: &str) -> Result<(), String> {
    // Setting and clearing a variable through .NET tells running programs
    // (Explorer, new terminals) that the environment changed.
    powershell(&format!(
        "Set-ItemProperty -LiteralPath 'HKCU:\\Environment' -Name Path -Value {} -Type {}\n\
         [Environment]::SetEnvironmentVariable('KEYJUTSU_ENV_REFRESH', '1', 'User')\n\
         [Environment]::SetEnvironmentVariable('KEYJUTSU_ENV_REFRESH', $null, 'User')",
        quote(value),
        quote(kind)
    ))
    .map(|_| ())
}

fn install_dir() -> Result<PathBuf, String> {
    std::env::current_exe()
        .map_err(|e| e.to_string())?
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| "cannot tell where keyjutsu.exe is".to_owned())
}

pub fn path(action: &str) -> ExitCode {
    let result = (|| -> Result<String, String> {
        let dir = install_dir()?.display().to_string();
        let (kind, current) = read_user_path()?;
        match action {
            "add" => {
                let next = path_with(&current, &dir);
                if next == current {
                    return Ok(format!("{dir} is already on your PATH."));
                }
                write_user_path(&kind, &next)?;
                Ok(format!("Added {dir} to your PATH. New terminals will find `keyjutsu`."))
            }
            "remove" => {
                let next = path_without(&current, &dir);
                if next == current {
                    return Ok(format!("{dir} was not on your PATH."));
                }
                write_user_path(&kind, &next)?;
                Ok(format!("Removed {dir} from your PATH."))
            }
            _ => Ok(if current.split(';').any(|e| norm(e) == norm(&dir)) {
                format!("{dir} is on your PATH.")
            } else {
                format!("{dir} is not on your PATH.")
            }),
        }
    })();
    report(result)
}

const MENU_KEYS: [&str; 2] = [
    "HKCU:\\Software\\Classes\\Directory\\shell\\KeyJutsu",
    "HKCU:\\Software\\Classes\\Directory\\Background\\shell\\KeyJutsu",
];

pub fn explorer(action: &str) -> ExitCode {
    let result = (|| -> Result<String, String> {
        match action {
            "add" => {
                let app = install_dir()?.join("keyjutsu-desktop.exe");
                if !app.exists() {
                    return Err(format!("{} is not installed", app.display()));
                }
                let app = app.display().to_string();
                let mut script = String::new();
                for key in MENU_KEYS {
                    script.push_str(&format!(
                        "New-Item -Path {k} -Force | Out-Null\n\
                         Set-ItemProperty -LiteralPath {k} -Name '(default)' -Value 'Open KeyJutsu here'\n\
                         Set-ItemProperty -LiteralPath {k} -Name Icon -Value {icon}\n\
                         New-Item -Path {c} -Force | Out-Null\n\
                         Set-ItemProperty -LiteralPath {c} -Name '(default)' -Value {cmd}\n",
                        k = quote(key),
                        c = quote(&format!("{key}\\command")),
                        icon = quote(&app),
                        cmd = quote(&format!("\"{app}\" --cwd \"%V\"")),
                    ));
                }
                powershell(&script)?;
                Ok("Added \"Open KeyJutsu here\" to the folder menus in Explorer.".into())
            }
            "remove" => {
                let script: String = MENU_KEYS
                    .iter()
                    .map(|k| {
                        format!(
                            "Remove-Item -LiteralPath {} -Recurse -Force -ErrorAction SilentlyContinue\n",
                            quote(k)
                        )
                    })
                    .collect();
                powershell(&script)?;
                Ok("Removed \"Open KeyJutsu here\" from Explorer.".into())
            }
            _ => {
                let here = powershell(&format!("Test-Path -LiteralPath {}", quote(MENU_KEYS[0])))?;
                Ok(if here.trim() == "True" {
                    "\"Open KeyJutsu here\" is in Explorer's folder menus.".into()
                } else {
                    "\"Open KeyJutsu here\" is not in Explorer.".into()
                })
            }
        }
    })();
    report(result)
}

fn report(result: Result<String, String>) -> ExitCode {
    match result {
        Ok(text) => {
            println!("{text}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("keyjutsu: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adding_is_idempotent_and_keeps_everything_else_as_written() {
        let path = r"%USERPROFILE%\bin;C:\Tools\";
        let added = path_with(path, r"C:\Program Files\KeyJutsu");
        assert_eq!(added, r"%USERPROFILE%\bin;C:\Tools\;C:\Program Files\KeyJutsu");
        assert_eq!(path_with(&added, r"c:\program files\keyjutsu\"), added, "already there, however written");
        assert_eq!(path_with("", r"C:\K"), r"C:\K");
        assert_eq!(path_with("C:\\A;", r"C:\K"), r"C:\A;C:\K");
    }

    #[test]
    fn removing_takes_out_only_that_folder() {
        let path = r"%USERPROFILE%\bin;C:\Program Files\KeyJutsu\;C:\Tools";
        assert_eq!(path_without(path, r"C:\Program Files\KeyJutsu"), r"%USERPROFILE%\bin;C:\Tools");
        assert_eq!(path_without(r"C:\A;;C:\B", r"C:\K"), r"C:\A;C:\B");
    }
}
