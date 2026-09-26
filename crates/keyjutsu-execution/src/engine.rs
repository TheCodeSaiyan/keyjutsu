//! The Performance Mode engine.
//!
//! A pure state machine: it takes inputs (keys, prompt marks, operator
//! actions, pacing ticks) and returns actions (bytes for the shell, state
//! changes, ticks to schedule). It does no I/O, which is what makes the
//! guarantees below testable without a terminal:
//!
//! - whatever the physical key, only the staged text reaches the shell;
//! - an incomplete staged command is never submitted;
//! - the hard-disarm chord always disarms, whatever state the engine is in;
//! - a step completes only when the shell reports that it finished, and a
//!   failure stops the performance rather than carrying on.

use std::time::Duration;

use keyjutsu_terminal::ShellMark;
use serde::{Deserialize, Serialize};

use crate::cadence::{Cadence, Humaniser};
use crate::input::{KeyClass, KeyInput};
use crate::script::{AdvanceStyle, ExecutionMode, ScriptError, StagedScript, SubmitPolicy};
use crate::state::ExecutionState::{self, *};

/// Erases one character of a partly typed line in every V1 shell.
const ERASE: u8 = 0x7f;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export)]
pub struct PerformanceConfig {
    /// The mode for steps that do not override it.
    pub mode: ExecutionMode,
    pub advance: AdvanceStyle,
    pub submit: SubmitPolicy,
    pub cadence: Cadence,
    /// Seed for Auto Performance's rhythm.
    pub seed: u32,
    /// Keep swallowing keys after the last step, so mashing on past the end of
    /// a performance does not spill junk into the shell. The disarm chord
    /// hands the terminal back.
    pub hold_input_after_complete: bool,
}

impl Default for PerformanceConfig {
    fn default() -> Self {
        Self {
            mode: ExecutionMode::Performance,
            advance: AdvanceStyle::Pure,
            submit: SubmitPolicy::AnyKey,
            cadence: Cadence::default(),
            seed: 0,
            hold_input_after_complete: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Input {
    /// The operator armed the performance. The caller has already checked
    /// that the shell is idle at an empty prompt.
    Arm,
    /// Begin the first step without waiting for a key.
    Start,
    Key(KeyInput),
    /// A pacing tick the engine asked for with [`Action::ScheduleTick`].
    Tick,
    Shell(ShellMark),
    Pause,
    Resume,
    /// Operator disarm from outside the key stream, such as an overlay button.
    Disarm,
    /// The shell process ended.
    ShellExited,
    /// Something the approval relied on changed.
    RequireRevalidation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize, ts_rs::TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export)]
pub enum StepOutcome {
    Succeeded {
        exit_code: i32,
    },
    /// The shell cannot report exit codes (cmd.exe), so success was not
    /// proven. Recorded as such rather than as a pass.
    Unverified,
    Failed {
        exit_code: i32,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Bytes for the shell's input.
    Write(Vec<u8>),
    StateChanged {
        from: ExecutionState,
        to: ExecutionState,
    },
    /// Deliver [`Input::Tick`] after this long. A newer request replaces it.
    ScheduleTick(Duration),
    /// Drop any pending tick.
    CancelTicks,
    StepStarted {
        index: usize,
    },
    StepFinished {
        index: usize,
        outcome: StepOutcome,
    },
    /// The engine no longer owns the keyboard: keys go to the shell again.
    Released,
    OverlayRequested,
}

/// What the operator overlay and the front ends show.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ts_rs::TS)]
#[ts(export)]
pub struct PerformanceSnapshot {
    pub state: ExecutionState,
    pub step_index: usize,
    pub step_count: usize,
    pub step_title: String,
    /// What comes after this step, for the operator overlay.
    pub next_step_title: Option<String>,
    pub step_mode: ExecutionMode,
    pub typed_chars: usize,
    pub total_chars: usize,
    pub owns_input: bool,
    /// This step asks the operator for something (a credential): staged
    /// typing is off, and it starts on Enter.
    pub asks_operator: bool,
    pub outcomes: Vec<StepOutcome>,
}

#[derive(Debug)]
pub struct PerformanceEngine {
    script: StagedScript,
    config: PerformanceConfig,
    state: ExecutionState,
    step: usize,
    chars: Vec<char>,
    typed: usize,
    /// Enter presses the operator has given the current line.
    answered: u32,
    paused_from: Option<ExecutionState>,
    pause_after_step: bool,
    released: bool,
    humaniser: Humaniser,
    outcomes: Vec<StepOutcome>,
    history: Vec<(ExecutionState, ExecutionState)>,
}

impl PerformanceEngine {
    pub fn new(script: StagedScript, config: PerformanceConfig) -> Result<Self, ScriptError> {
        script.validate(config.mode)?;
        let chars = script.steps[0].command.chars().collect();
        Ok(Self {
            humaniser: Humaniser::new(u64::from(config.seed)),
            script,
            config,
            state: Preparing,
            step: 0,
            chars,
            typed: 0,
            answered: 0,
            paused_from: None,
            pause_after_step: false,
            released: false,
            outcomes: Vec::new(),
            history: Vec::new(),
        })
    }

    pub fn state(&self) -> ExecutionState {
        self.state
    }

    /// Every state change so far, in order.
    pub fn history(&self) -> &[(ExecutionState, ExecutionState)] {
        &self.history
    }

    /// Whether keys belong to the engine rather than the shell.
    pub fn owns_input(&self) -> bool {
        !self.released && !matches!(self.state, Preparing | Failed | RevalidationRequired | Aborted)
    }

    pub fn snapshot(&self) -> PerformanceSnapshot {
        let step = &self.script.steps[self.step];
        PerformanceSnapshot {
            state: self.state,
            step_index: self.step,
            step_count: self.script.steps.len(),
            step_title: step.title.clone(),
            next_step_title: self.script.steps.get(self.step + 1).map(|s| s.title.clone()),
            step_mode: self.mode(),
            typed_chars: self.typed,
            total_chars: self.chars.len(),
            asks_operator: self.asks_operator(),
            owns_input: self.owns_input(),
            outcomes: self.outcomes.clone(),
        }
    }

    fn mode(&self) -> ExecutionMode {
        self.script.steps[self.step].mode.unwrap_or(self.config.mode)
    }

    fn submit_policy(&self) -> SubmitPolicy {
        match self.mode() {
            ExecutionMode::AutoPerformance => SubmitPolicy::AutoSubmit,
            _ => self.script.steps[self.step].submit.unwrap_or(self.config.submit),
        }
    }

    pub fn handle(&mut self, input: Input) -> Vec<Action> {
        let mut out = Vec::new();
        match input {
            Input::Arm => self.arm(&mut out),
            // A line that asks the operator starts only on their Enter.
            Input::Start if self.state == Armed && !self.asks_operator() => self.enter_step(&mut out),
            Input::Start => {}
            Input::Key(key) => self.key(key, &mut out),
            Input::Tick => self.tick(&mut out),
            Input::Shell(mark) => self.shell_mark(mark, &mut out),
            Input::Pause => self.pause(&mut out),
            Input::Resume => self.resume(&mut out),
            Input::Disarm => self.disarm(&mut out),
            Input::ShellExited => {
                if !self.state.is_terminal() && self.state != Failed {
                    out.push(Action::CancelTicks);
                    self.set(Failed, &mut out);
                    self.release(&mut out);
                }
            }
            Input::RequireRevalidation => {
                if self.state.can_transition_to(RevalidationRequired) {
                    self.erase_partial_line(&mut out);
                    out.push(Action::CancelTicks);
                    self.set(RevalidationRequired, &mut out);
                    self.release(&mut out);
                }
            }
        }
        out
    }

    fn arm(&mut self, out: &mut Vec<Action>) {
        if self.state != Preparing {
            return;
        }
        self.set(Armed, out);
        if self.mode() == ExecutionMode::AutoPerformance {
            out.push(Action::ScheduleTick(self.humaniser.boundary(&self.config.cadence)));
        }
    }

    fn key(&mut self, key: KeyInput, out: &mut Vec<Action>) {
        if key.class == KeyClass::HardDisarm {
            // Checked before ownership, so a disarm is never lost.
            self.disarm(out);
            return;
        }
        if !self.owns_input() {
            return;
        }
        let forward = |out: &mut Vec<Action>| {
            if let Some(bytes) = &key.bytes {
                out.push(Action::Write(bytes.clone()));
            }
        };
        match (key.class, self.state) {
            (KeyClass::Overlay, _) => out.push(Action::OverlayRequested),

            // A user-input step is the operator typing for real. Once they
            // have given every answer the line asks for, the command runs
            // and later keys are swallowed like any other running command.
            (class, AwaitingUserInput) => {
                forward(out);
                if class == KeyClass::Enter && self.asks_operator() {
                    self.answered += 1;
                    let answers = self.script.steps[self.step].answers;
                    if answers.is_some_and(|n| self.answered >= n) {
                        self.set(Executing, out);
                    }
                }
            }

            // Ctrl+C is always a real interrupt. If it lands while a staged
            // line is being typed, the shell abandons that line, so the step
            // pauses and will be retyped from the start on resume.
            (KeyClass::Interrupt, Typing | AwaitingExecution) => {
                forward(out);
                out.push(Action::CancelTicks);
                self.typed = 0;
                self.paused_from = Some(Typing);
                self.set(Paused, out);
            }
            (KeyClass::Interrupt, _) => forward(out),

            // Esc reaches a running program, but is swallowed while KeyJutsu
            // owns the input line: the shell would clear the staged text.
            (KeyClass::Escape, Executing | Waiting) => forward(out),
            (KeyClass::Escape, _) => {}

            // A line that asks the operator for something starts only on
            // Enter: keys still being mashed must not land in the prompt.
            (KeyClass::Advance, Armed) if self.asks_operator() => {}
            (KeyClass::Advance | KeyClass::Enter, Armed) => {
                self.enter_step(out);
                if matches!(self.mode(), ExecutionMode::Performance | ExecutionMode::Assisted) {
                    self.advance_by_key(out);
                }
            }
            (KeyClass::Advance | KeyClass::Enter, Typing) => {
                if self.mode() != ExecutionMode::AutoPerformance {
                    self.advance_by_key(out);
                }
            }
            (class @ (KeyClass::Advance | KeyClass::Enter), AwaitingExecution) => {
                let submits = match self.submit_policy() {
                    SubmitPolicy::AnyKey => true,
                    SubmitPolicy::RequireEnter => class == KeyClass::Enter,
                    SubmitPolicy::AutoSubmit => false,
                };
                if submits {
                    self.submit(out);
                }
            }
            // Everything else is swallowed: while a command runs, mashing
            // must not feed the running program.
            _ => {}
        }
    }

    fn tick(&mut self, out: &mut Vec<Action>) {
        if self.released {
            return;
        }
        let auto = self.mode() == ExecutionMode::AutoPerformance;
        match self.state {
            Armed if auto => self.enter_step(out),
            Typing if auto => {
                self.advance(1, out);
                if self.state == Typing {
                    let just_typed = self.chars[self.typed - 1];
                    out.push(Action::ScheduleTick(
                        self.humaniser.delay_after(&self.config.cadence, just_typed),
                    ));
                }
            }
            AwaitingExecution if auto => self.submit(out),
            _ => {}
        }
    }

    fn shell_mark(&mut self, mark: ShellMark, out: &mut Vec<Action>) {
        let ShellMark::CommandFinished { exit_code } = mark else { return };
        if !matches!(self.state, Executing | Waiting | AwaitingUserInput) {
            return;
        }
        self.set(Validating, out);
        let outcome = match exit_code {
            Some(0) => StepOutcome::Succeeded { exit_code: 0 },
            Some(code) => StepOutcome::Failed { exit_code: code },
            None => StepOutcome::Unverified,
        };
        self.outcomes.push(outcome);
        out.push(Action::StepFinished { index: self.step, outcome });

        if matches!(outcome, StepOutcome::Failed { .. }) {
            self.set(Failed, out);
            self.release(out);
            return;
        }
        if self.step + 1 == self.script.steps.len() {
            self.set(Complete, out);
            if !self.config.hold_input_after_complete {
                self.release(out);
            }
            return;
        }
        self.step += 1;
        self.chars = self.script.steps[self.step].command.chars().collect();
        self.typed = 0;
        if self.pause_after_step {
            self.pause_after_step = false;
            self.paused_from = None;
            self.set(Paused, out);
        } else {
            self.enter_step(out);
        }
    }

    fn pause(&mut self, out: &mut Vec<Action>) {
        match self.state {
            Armed | Typing | AwaitingExecution | AwaitingUserInput => {
                out.push(Action::CancelTicks);
                self.paused_from = Some(self.state);
                self.set(Paused, out);
            }
            // A running command cannot be paused; stop before the next step.
            Executing | Waiting | Validating => self.pause_after_step = true,
            _ => {}
        }
    }

    fn resume(&mut self, out: &mut Vec<Action>) {
        if self.state != Paused {
            self.pause_after_step = false;
            return;
        }
        match self.paused_from.take() {
            // Paused between steps: the next step has not started yet.
            None => self.enter_step(out),
            Some(previous) => {
                self.set(previous, out);
                if self.mode() == ExecutionMode::AutoPerformance
                    && matches!(previous, Armed | Typing | AwaitingExecution)
                {
                    out.push(Action::ScheduleTick(self.humaniser.boundary(&self.config.cadence)));
                }
            }
        }
    }

    fn disarm(&mut self, out: &mut Vec<Action>) {
        out.push(Action::CancelTicks);
        self.erase_partial_line(out);
        if !self.state.is_terminal() {
            self.set(Aborted, out);
        }
        self.release(out);
    }

    fn enter_step(&mut self, out: &mut Vec<Action>) {
        out.push(Action::StepStarted { index: self.step });
        self.typed = 0;
        self.answered = 0;
        match self.mode() {
            ExecutionMode::Performance | ExecutionMode::Assisted => self.set(Typing, out),
            ExecutionMode::AutoPerformance => {
                self.set(Typing, out);
                out.push(Action::ScheduleTick(self.humaniser.boundary(&self.config.cadence)));
            }
            ExecutionMode::Direct => {
                let mut bytes = self.script.steps[self.step].command.clone().into_bytes();
                bytes.push(b'\r');
                self.typed = self.chars.len();
                out.push(Action::Write(bytes));
                self.set(Executing, out);
            }
            // A user-input line with a command is KeyJutsu's command and the
            // operator's answer: the command is written directly (never
            // performed), then the keys are the operator's until the
            // shell reports it finished. How a credential is asked for.
            ExecutionMode::UserInput => {
                if !self.chars.is_empty() {
                    let mut bytes = self.script.steps[self.step].command.clone().into_bytes();
                    bytes.push(b'\r');
                    self.typed = self.chars.len();
                    out.push(Action::Write(bytes));
                }
                self.set(AwaitingUserInput, out);
            }
        }
    }

    /// A user-input line with a command: KeyJutsu runs the command and the
    /// operator answers it, as a credential step does.
    pub fn asks_operator(&self) -> bool {
        self.mode() == ExecutionMode::UserInput && !self.chars.is_empty()
    }

    fn advance_by_key(&mut self, out: &mut Vec<Action>) {
        let n = match (self.mode(), self.config.advance) {
            (ExecutionMode::Assisted, _) => self.burst_len(),
            (_, AdvanceStyle::Turbo) => self.word_len(),
            (_, AdvanceStyle::Pure) => 1,
        };
        self.advance(n, out);
    }

    /// Up to three characters, stopping after whitespace so words land whole.
    fn burst_len(&self) -> usize {
        let mut n = 0;
        while self.typed + n < self.chars.len() && n < 3 {
            n += 1;
            if self.chars[self.typed + n - 1].is_whitespace() {
                break;
            }
        }
        n
    }

    /// The rest of the current word and the whitespace after it.
    fn word_len(&self) -> usize {
        let rest = &self.chars[self.typed..];
        let word = rest.iter().take_while(|c| !c.is_whitespace()).count();
        let space = rest[word..].iter().take_while(|c| c.is_whitespace()).count();
        (word + space).max(1)
    }

    fn advance(&mut self, n: usize, out: &mut Vec<Action>) {
        let end = (self.typed + n).min(self.chars.len());
        if end > self.typed {
            let text: String = self.chars[self.typed..end].iter().collect();
            out.push(Action::Write(text.into_bytes()));
            self.typed = end;
        }
        if self.typed == self.chars.len() {
            if self.submit_policy() == SubmitPolicy::AutoSubmit
                && self.mode() != ExecutionMode::AutoPerformance
            {
                self.submit(out);
            } else {
                self.set(AwaitingExecution, out);
                if self.mode() == ExecutionMode::AutoPerformance {
                    out.push(Action::ScheduleTick(self.humaniser.boundary(&self.config.cadence)));
                }
            }
        }
    }

    fn submit(&mut self, out: &mut Vec<Action>) {
        // The only place a staged Enter is produced, and only once every
        // character of the command has been delivered.
        debug_assert_eq!(self.typed, self.chars.len());
        out.push(Action::Write(b"\r".to_vec()));
        self.set(Executing, out);
    }

    fn erase_partial_line(&mut self, out: &mut Vec<Action>) {
        // A pause taken while typing leaves the staged text on the line. A
        // pause caused by Ctrl+C does not: the shell dropped the line and
        // `typed` was reset when it happened.
        let line_holds_staged_text = match self.state {
            Typing | AwaitingExecution => true,
            Paused => matches!(self.paused_from, Some(Typing | AwaitingExecution)),
            _ => false,
        };
        if line_holds_staged_text && self.typed > 0 {
            out.push(Action::Write(vec![ERASE; self.typed]));
            self.typed = 0;
        }
    }

    fn release(&mut self, out: &mut Vec<Action>) {
        if !self.released {
            self.released = true;
            out.push(Action::Released);
        }
    }

    fn set(&mut self, to: ExecutionState, out: &mut Vec<Action>) {
        let from = self.state;
        if from == to {
            return;
        }
        assert!(from.can_transition_to(to), "illegal execution transition {from:?} -> {to:?}");
        self.state = to;
        self.history.push((from, to));
        out.push(Action::StateChanged { from, to });
    }
}
