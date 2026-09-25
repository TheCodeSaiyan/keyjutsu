//! The execution state machine.
//!
//! Every state change the engine makes is checked against this table, and the
//! table is the one documented in `docs/architecture/execution-state-machine.md`.
//! States change because something real happened (a key, a prompt mark from
//! the shell, the shell exiting, an operator action), never because a timer
//! ran out. Timers only pace what Auto Performance *shows*.

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, ts_rs::TS)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[ts(export)]
pub enum ExecutionState {
    /// Staged input exists but has not been armed.
    Preparing,
    /// Armed and waiting for the performance to begin.
    Armed,
    /// Staged characters are being delivered to the shell's input line.
    Typing,
    /// The whole command is on the input line; waiting for the submit key.
    AwaitingExecution,
    /// Submitted; waiting for the shell to report the command finished.
    Executing,
    /// Waiting on an external condition after a command, such as a service
    /// becoming healthy. Entered by runtime validation (Milestone 8).
    Waiting,
    /// Checking the finished command against its validation contract.
    Validating,
    /// Keys go to the shell for real: a user-input or credential step.
    AwaitingUserInput,
    Paused,
    /// Reality differed from the plan. Control returns to the operator.
    Failed,
    /// Something the approval relied on has changed.
    RevalidationRequired,
    Complete,
    Aborted,
}

use ExecutionState::*;

impl ExecutionState {
    pub const ALL: [ExecutionState; 13] = [
        Preparing,
        Armed,
        Typing,
        AwaitingExecution,
        Executing,
        Waiting,
        Validating,
        AwaitingUserInput,
        Paused,
        Failed,
        RevalidationRequired,
        Complete,
        Aborted,
    ];

    /// No transition leaves these.
    pub fn is_terminal(self) -> bool {
        matches!(self, Complete | Aborted)
    }

    /// The legal successors of each state.
    pub fn successors(self) -> &'static [ExecutionState] {
        match self {
            Preparing => &[Armed, Failed, RevalidationRequired, Aborted],
            Armed => &[Typing, Executing, AwaitingUserInput, Paused, Failed, RevalidationRequired, Aborted],
            Typing => &[AwaitingExecution, Executing, Paused, Failed, RevalidationRequired, Aborted],
            AwaitingExecution => &[Executing, Paused, Failed, RevalidationRequired, Aborted],
            Executing => &[Waiting, Validating, Failed, Aborted],
            Waiting => &[Validating, Failed, Aborted],
            Validating => &[Typing, Executing, AwaitingUserInput, Paused, Complete, Failed, Aborted],
            AwaitingUserInput => &[Validating, Executing, Paused, Failed, Aborted],
            Paused => &[
                Armed,
                Typing,
                AwaitingExecution,
                Executing,
                AwaitingUserInput,
                Failed,
                RevalidationRequired,
                Aborted,
            ],
            Failed => &[RevalidationRequired, Aborted],
            RevalidationRequired => &[Preparing, Aborted],
            Complete | Aborted => &[],
        }
    }

    pub fn can_transition_to(self, to: ExecutionState) -> bool {
        self.successors().contains(&to)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_states_have_no_successors_and_others_do() {
        for s in ExecutionState::ALL {
            assert_eq!(s.is_terminal(), s.successors().is_empty(), "{s:?}");
        }
    }

    #[test]
    fn every_non_terminal_state_can_be_aborted() {
        for s in ExecutionState::ALL.into_iter().filter(|s| !s.is_terminal()) {
            assert!(s.can_transition_to(Aborted), "{s:?} cannot be aborted");
        }
    }

    #[test]
    fn a_command_cannot_be_paused_mid_execution() {
        // The process runs regardless of what KeyJutsu shows, so pretending to
        // pause it would misrepresent the machine's state. A pause requested
        // while executing takes effect after validation.
        assert!(!Executing.can_transition_to(Paused));
        assert!(Validating.can_transition_to(Paused));
    }

    #[test]
    fn failure_never_resumes_without_revalidation() {
        assert_eq!(Failed.successors(), &[RevalidationRequired, Aborted]);
        assert_eq!(RevalidationRequired.successors(), &[Preparing, Aborted]);
    }

    #[test]
    fn every_state_is_reachable_from_preparing() {
        let mut seen = vec![Preparing];
        let mut frontier = vec![Preparing];
        while let Some(s) = frontier.pop() {
            for &n in s.successors() {
                if !seen.contains(&n) {
                    seen.push(n);
                    frontier.push(n);
                }
            }
        }
        for s in ExecutionState::ALL {
            assert!(seen.contains(&s), "{s:?} is unreachable");
        }
    }

    #[test]
    fn serialises_as_the_names_the_specification_uses() {
        assert_eq!(serde_json::to_string(&AwaitingExecution).unwrap(), "\"AWAITING_EXECUTION\"");
        assert_eq!(serde_json::to_string(&RevalidationRequired).unwrap(), "\"REVALIDATION_REQUIRED\"");
    }
}
