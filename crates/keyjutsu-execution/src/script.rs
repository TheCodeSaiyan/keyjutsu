//! Staged input: the exact commands a performance will deliver.
//!
//! Until approved plan snapshots exist (Milestones 3, 4 and 8), a staged
//! script is built from the built-in read-only demo or from commands the
//! operator types themselves. Either way the engine treats it as fixed: it is
//! validated once when the engine is created and never modified afterwards.

use serde::{Deserialize, Serialize};

/// How a step is delivered. The global mode applies unless a step overrides it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ExecutionMode {
    /// Each ordinary physical key advances the staged command.
    #[default]
    Performance,
    /// Each key advances a small burst, stopping at word boundaries.
    Assisted,
    /// KeyJutsu types, submits and validates by itself. Watch-only, and it
    /// carries no authority the other modes lack.
    AutoPerformance,
    /// No simulated typing: the command is sent whole.
    Direct,
    /// Keys go to the shell for real, for steps that need the operator.
    UserInput,
}

/// How far one key advances in [`ExecutionMode::Performance`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum AdvanceStyle {
    /// One key, one character.
    #[default]
    Pure,
    /// One key, one word and the whitespace after it.
    Turbo,
}

/// What submits a fully typed command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum SubmitPolicy {
    /// Any advancing key delivers the staged Enter.
    #[default]
    AnyKey,
    /// Only a real Enter submits.
    RequireEnter,
    /// Submit as soon as the last character is typed.
    AutoSubmit,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export)]
pub struct StagedStep {
    pub id: String,
    pub title: String,
    /// The exact text delivered to the shell's input line. Empty only for a
    /// user-input step.
    pub command: String,
    #[serde(default)]
    pub mode: Option<ExecutionMode>,
    #[serde(default)]
    pub submit: Option<SubmitPolicy>,
    /// For a line that asks the operator: how many times their answer is
    /// ended with Enter (a user name and a password is two). After the last,
    /// keys stop reaching the shell until the command finishes, so nothing
    /// typed afterwards lands on the next prompt. `None` forwards keys until
    /// the command finishes.
    #[serde(default)]
    pub answers: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, ts_rs::TS)]
#[ts(export)]
pub struct StagedScript {
    pub steps: Vec<StagedStep>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ScriptError {
    #[error("there is nothing to perform")]
    Empty,
    #[error("step `{0}` has no command")]
    EmptyCommand(String),
    #[error(
        "step `{step}` contains a control character at position {index}; staged commands are a single line of text"
    )]
    ControlCharacter { step: String, index: usize },
    #[error("step id `{0}` is used more than once")]
    DuplicateId(String),
    #[error(
        "step `{0}` asks the operator for input part-way through a script; such a step must come first, so it waits for the operator rather than catching keys still being pressed for the step before"
    )]
    OperatorLineNotFirst(String),
}

impl StagedScript {
    /// Reject anything the engine could not deliver faithfully. A newline or
    /// other control character would submit or edit the line part-way through
    /// a command, which is exactly the accidental submission the engine exists
    /// to prevent.
    pub fn validate(&self, default_mode: ExecutionMode) -> Result<(), ScriptError> {
        if self.steps.is_empty() {
            return Err(ScriptError::Empty);
        }
        let mut seen = std::collections::HashSet::new();
        for (i, step) in self.steps.iter().enumerate() {
            if !seen.insert(step.id.as_str()) {
                return Err(ScriptError::DuplicateId(step.id.clone()));
            }
            let mode = step.mode.unwrap_or(default_mode);
            if step.command.is_empty() && mode != ExecutionMode::UserInput {
                return Err(ScriptError::EmptyCommand(step.id.clone()));
            }
            if let Some(index) = step.command.chars().position(char::is_control) {
                return Err(ScriptError::ControlCharacter { step: step.id.clone(), index });
            }
            if i > 0 && mode == ExecutionMode::UserInput && !step.command.is_empty() {
                return Err(ScriptError::OperatorLineNotFirst(step.id.clone()));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(id: &str, command: &str) -> StagedStep {
        StagedStep {
            id: id.into(),
            title: id.into(),
            command: command.into(),
            mode: None,
            submit: None,
            answers: None,
        }
    }

    #[test]
    fn rejects_scripts_that_could_submit_part_of_a_command() {
        let s = StagedScript { steps: vec![step("a", "Get-Date\rRemove-Item x")] };
        assert_eq!(
            s.validate(ExecutionMode::Performance),
            Err(ScriptError::ControlCharacter { step: "a".into(), index: 8 })
        );
        let tab = StagedScript { steps: vec![step("a", "Get-\tDate")] };
        assert!(tab.validate(ExecutionMode::Performance).is_err());
    }

    #[test]
    fn rejects_empty_and_duplicate_steps() {
        assert_eq!(StagedScript::default().validate(ExecutionMode::Performance), Err(ScriptError::Empty));
        let e = StagedScript { steps: vec![step("a", "")] };
        assert_eq!(e.validate(ExecutionMode::Performance), Err(ScriptError::EmptyCommand("a".into())));
        let d = StagedScript { steps: vec![step("a", "x"), step("a", "y")] };
        assert_eq!(d.validate(ExecutionMode::Performance), Err(ScriptError::DuplicateId("a".into())));
    }

    #[test]
    fn a_line_that_asks_the_operator_must_come_first() {
        let mut ask = step("ask", "$t = Read-Host -AsSecureString");
        ask.mode = Some(ExecutionMode::UserInput);
        let first = StagedScript { steps: vec![ask.clone(), step("use", "Get-Date")] };
        assert_eq!(first.validate(ExecutionMode::Performance), Ok(()));
        let later = StagedScript { steps: vec![step("before", "Get-Date"), ask] };
        assert_eq!(
            later.validate(ExecutionMode::Performance),
            Err(ScriptError::OperatorLineNotFirst("ask".into()))
        );
    }

    #[test]
    fn a_user_input_step_may_have_no_command() {
        let mut s = step("a", "");
        s.mode = Some(ExecutionMode::UserInput);
        assert_eq!(StagedScript { steps: vec![s] }.validate(ExecutionMode::Performance), Ok(()));
    }
}
