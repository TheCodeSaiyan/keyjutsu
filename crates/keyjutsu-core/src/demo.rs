//! The safe demo: a short, read-only performance that shows the whole idea on
//! a machine nobody has planned anything for yet.
//!
//! Every command here only reads. The specification suggests
//! `Get-PSVersionTable`, which is not a cmdlet; the demo reads the
//! `$PSVersionTable` variable instead, and the deviation is recorded in
//! `docs/architecture/deviations.md`.

use keyjutsu_execution::{StagedScript, StagedStep};
use keyjutsu_terminal::ShellKind;

fn step(id: &str, title: &str, command: &str) -> StagedStep {
    StagedStep { id: id.into(), title: title.into(), command: command.into(), mode: None, submit: None }
}

pub fn safe_demo(shell: ShellKind) -> StagedScript {
    let steps = match shell {
        ShellKind::Pwsh | ShellKind::WindowsPowershell => vec![
            step(
                "computer",
                "Describe this computer",
                "Get-ComputerInfo -Property OsName, OsVersion, OsArchitecture, CsNumberOfLogicalProcessors",
            ),
            step(
                "powershell",
                "Show the PowerShell version",
                "$PSVersionTable | Select-Object PSVersion, PSEdition",
            ),
            step(
                "volumes",
                "List the volumes",
                "Get-Volume | Where-Object DriveLetter | Sort-Object DriveLetter | Format-Table DriveLetter, FileSystemLabel, FileSystem, SizeRemaining, Size -AutoSize",
            ),
        ],
        ShellKind::Cmd => vec![
            step("version", "Show the Windows version", "ver"),
            step("arch", "Show the processor architecture", "echo %PROCESSOR_ARCHITECTURE%"),
            step("volume", "Describe drive C:", "vol C:"),
        ],
    };
    StagedScript { steps }
}

#[cfg(test)]
mod tests {
    use super::*;
    use keyjutsu_execution::ExecutionMode;

    #[test]
    fn the_demo_is_valid_for_every_shell() {
        for shell in ShellKind::ALL {
            assert_eq!(safe_demo(shell).validate(ExecutionMode::Performance), Ok(()), "{shell:?}");
        }
    }

    #[test]
    fn the_demo_only_uses_read_only_verbs() {
        // A blunt guard against someone "improving" the demo into something
        // that changes the machine.
        let allowed = [
            "Get-",
            "$PSVersionTable",
            "Select-Object",
            "Where-Object",
            "Sort-Object",
            "Format-Table",
            "ver",
            "echo",
            "vol",
        ];
        for shell in ShellKind::ALL {
            for s in safe_demo(shell).steps {
                for segment in s.command.split('|') {
                    let first = segment.trim();
                    assert!(allowed.iter().any(|a| first.starts_with(a)), "{first}");
                }
            }
        }
    }
}
