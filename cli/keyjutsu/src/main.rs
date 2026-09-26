//! `keyjutsu`: the command-line front end.
//!
//! Everything here is presentation and argument parsing. Sessions, staged
//! input and the rules about what reaches a shell come from keyjutsu-core,
//! the same code the desktop app uses.

mod agent_cli;
mod console;
mod git_cli;
mod history_cli;
mod plans;
mod recover_cli;
mod run_cli;
mod setup_cli;

use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};
use keyjutsu_core::execution::{
    AdvanceStyle, ExecutionMode, PerformanceConfig, StagedScript, StagedStep, StepOutcome, SubmitPolicy,
};
use keyjutsu_core::readiness::{self, CheckStatus};
use keyjutsu_core::terminal::{ProfileMode, ShellKind};
use keyjutsu_core::{SessionOptions, demo};

/// "1 step", "2 steps": a count as a person would say it.
pub(crate) fn count(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

#[derive(Parser)]
#[command(name = "keyjutsu", version, about = "Validated commands, theatrically typed.", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum SetupCommand {
    /// Put the folder holding keyjutsu.exe on your PATH, or take it off.
    Path {
        #[arg(value_enum)]
        action: SetupAction,
    },
    /// "Open KeyJutsu here" on folders in Explorer.
    Explorer {
        #[arg(value_enum)]
        action: SetupAction,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum SetupAction {
    Add,
    Remove,
    Status,
}

impl SetupAction {
    fn as_str(self) -> &'static str {
        match self {
            Self::Add => "add",
            Self::Remove => "remove",
            Self::Status => "status",
        }
    }
}

#[derive(Subcommand)]
enum HistoryCommand {
    /// Every recorded session, oldest first.
    List,
    /// One session: its task, agent, outcome and steps.
    Show {
        /// The session's id, from `keyjutsu history list`.
        id: String,
    },
    /// Compare this machine with the one the session was approved on.
    Recheck {
        /// The session's id, from `keyjutsu history list`.
        id: String,
    },
}

#[derive(Subcommand)]
enum TechniqueCommand {
    /// Make a completed session into a Technique.
    Promote {
        /// The session's id, from `keyjutsu history list`. It must have completed.
        session: String,
        /// What to call the Technique.
        #[arg(long)]
        name: String,
        /// What it is for, in a sentence.
        #[arg(long, default_value = "")]
        description: String,
        /// A value in the session's plan to make a parameter: NAME=VALUE.
        #[arg(long = "param", value_name = "NAME=VALUE")]
        params: Vec<String>,
    },
    /// Every saved Technique, with its revision and parameters.
    List,
    /// Make a draft plan from a Technique, to validate and approve.
    Use {
        /// The Technique's id, from `keyjutsu technique list`.
        id: String,
        /// A value for one of its parameters: NAME=VALUE. Parameters with a
        /// default may be left out.
        #[arg(long = "param", value_name = "NAME=VALUE")]
        params: Vec<String>,
        /// Where to write the draft plan.
        #[arg(long)]
        out: std::path::PathBuf,
    },
    /// Save an adapted template as a new revision; earlier ones are kept.
    Revise {
        /// The Technique's id.
        id: String,
        /// A plan file to become the new template.
        #[arg(long)]
        template: std::path::PathBuf,
    },
    /// Write a Technique for sharing, without this machine's details.
    Export {
        /// The Technique's id.
        id: String,
        /// Where to write the export.
        #[arg(long)]
        out: std::path::PathBuf,
    },
    /// Read a shared Technique as an untrusted draft.
    Import {
        /// A file written by `keyjutsu technique export`.
        file: std::path::PathBuf,
    },
}

#[derive(Subcommand)]
enum StoreCommand {
    /// Delete what KeyJutsu keeps. Say which; nothing is cleared by default.
    Clear {
        /// Every recorded session.
        #[arg(long)]
        history: bool,
        /// Every Technique and all its revisions.
        #[arg(long)]
        techniques: bool,
        /// Every staged download.
        #[arg(long)]
        artifacts: bool,
    },
}

#[derive(Subcommand)]
enum DiagnosticsCommand {
    /// Print the bundle: everything a save would write, and nothing else.
    Preview,
    /// Write the bundle to a plain text file, to read before sending it.
    Save {
        /// Where to write it.
        file: std::path::PathBuf,
        /// Replace an existing file.
        #[arg(long)]
        force: bool,
    },
}

#[derive(Subcommand)]
enum GitCommand {
    /// KeyJutsu's changes alone, against each file as it was just before the run.
    Diff {
        /// The snapshot the run was made from.
        snapshot: std::path::PathBuf,
    },
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum IsolateChoice {
    /// A temporary worktree on a new local branch; your working tree is untouched.
    Worktree,
    /// A new local branch, switched to in place; your uncommitted changes come along.
    Branch,
}

#[derive(Subcommand)]
enum Command {
    /// List the supported AI agents: installed, version, sign-in, read-only mode.
    Agents {
        #[command(subcommand)]
        action: Option<AgentsCommand>,
        /// Print the list as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Show how to undo what a stopped run changed, and with --confirm, do it.
    Recover {
        /// The approved snapshot the run was made from.
        snapshot: std::path::PathBuf,
        /// The checkpoint of the run to recover; the one next to the snapshot otherwise.
        #[arg(long, value_name = "CHECKPOINT")]
        from: Option<std::path::PathBuf>,
        /// Recover only these steps (repeatable); every step that ran otherwise.
        #[arg(long = "step", value_name = "STEP")]
        steps: Vec<String>,
        /// Carry out the recovery plan. Without it, the plan is only shown.
        #[arg(long)]
        confirm: bool,
        /// Skip your shell profile for any recovery commands.
        #[arg(long)]
        clean: bool,
    },
    /// Execute an approved snapshot in this console.
    Run {
        /// An approved snapshot, from `keyjutsu plan approve`.
        snapshot: std::path::PathBuf,
        /// How steps without their own mode are delivered; the plan's default otherwise.
        #[arg(long, value_enum)]
        mode: Option<ModeChoice>,
        /// Skip your shell profile, history predictions and history saving.
        #[arg(long)]
        clean: bool,
        /// Continue from a checkpoint: the one next to the snapshot, or the given
        /// file (such as the previous snapshot's, after a revision). Steps that
        /// succeeded and are unchanged are not run again.
        #[arg(long, value_name = "CHECKPOINT", num_args = 0..=1)]
        resume: Option<Option<std::path::PathBuf>>,
        /// Settle a step left in doubt by a crash: STEP=succeeded or STEP=failed.
        #[arg(long, value_name = "STEP=RESULT")]
        settle: Vec<String>,
        /// Run apart from your working tree: in a new worktree, or on a new branch.
        #[arg(long, value_enum)]
        isolate: Option<IsolateChoice>,
        /// Keep no record of this session in the encrypted history.
        #[arg(long)]
        ephemeral: bool,
        /// How KeyJutsu asks for you during the run: on screen, or only in
        /// the window's title bar, to keep the illusion.
        #[arg(long, value_enum, default_value = "standard")]
        presentation: PresentationChoice,
    },
    /// What the installer offers: `keyjutsu` on your PATH, and "Open KeyJutsu
    /// here" in Explorer. Each changes only your Windows account.
    #[command(subcommand)]
    Setup(SetupCommand),
    /// The encrypted history of past sessions.
    #[command(subcommand)]
    History(HistoryCommand),
    /// Reusable Techniques made from successful sessions.
    #[command(subcommand)]
    Technique(TechniqueCommand),
    /// Manage what KeyJutsu keeps on this machine.
    #[command(subcommand)]
    Store(StoreCommand),
    /// What a run did to the Git repositories it worked in.
    #[command(subcommand)]
    Git(GitCommand),
    /// Check this machine is ready: Windows, ConPTY, shells and staged input.
    Doctor {
        /// Print the full report as JSON.
        #[arg(long)]
        json: bool,
    },
    /// A diagnostic bundle for someone helping you: versions, checks and
    /// counts, with no tasks, commands, output or secrets. Nothing is sent.
    #[command(subcommand)]
    Diagnostics(DiagnosticsCommand),
    /// Open an ordinary interactive shell through KeyJutsu's terminal.
    Shell(ShellArgs),
    /// Run the safe, read-only demo performance.
    Demo {
        #[command(flatten)]
        shell: ShellArgs,
        #[command(flatten)]
        performance: PerformanceArgs,
    },
    /// Work with plan files.
    #[command(subcommand)]
    Plan(PlanCommand),
    /// Perform commands you supply. They are yours: no plan, no validation and
    /// no approval, and they run exactly as if you had typed them.
    Perform {
        /// A command to stage. Repeat for several steps, run in order.
        #[arg(short = 'c', long = "command", required = true)]
        commands: Vec<String>,
        #[command(flatten)]
        shell: ShellArgs,
        #[command(flatten)]
        performance: PerformanceArgs,
    },
}

#[derive(Subcommand)]
enum AgentsCommand {
    /// Send each installed agent a tiny request and check the adapter still
    /// works with its version. This uses your agents' accounts.
    Check {
        /// Required: confirms you mean to send requests to the agents.
        #[arg(long)]
        live: bool,
    },
}

#[derive(clap::Args)]
struct AgentArgs {
    /// Which agent: codex, claude, gemini, copilot or cursor.
    #[arg(long)]
    agent: String,
    /// Actually send the request. Without it, KeyJutsu shows what would be
    /// sent and stops.
    #[arg(long)]
    send: bool,
}

#[derive(Subcommand)]
enum PlanCommand {
    /// Ask an agent to investigate (read-only) and propose a plan.
    Propose {
        /// What you want done.
        task: String,
        #[command(flatten)]
        agent: AgentArgs,
        /// A file to include, redacted. Repeatable.
        #[arg(long = "file")]
        files: Vec<std::path::PathBuf>,
        /// A folder for the agent to investigate; it is started there.
        #[arg(long)]
        folder: Option<std::path::PathBuf>,
        /// Where to write the proposed plan.
        #[arg(long)]
        out: std::path::PathBuf,
    },
    /// Ask an agent to revise one step, with your guidance.
    Revise {
        /// The plan to revise.
        file: std::path::PathBuf,
        /// The id of the step to redo; no other step may change.
        #[arg(long)]
        step: String,
        /// What you want different about it.
        #[arg(long)]
        guidance: String,
        /// The task, if the plan's title does not say it well enough.
        #[arg(long)]
        task: Option<String>,
        /// A recorded run in which this step failed: the agent is shown what
        /// it printed, redacted, to diagnose from. Its id is in
        /// `keyjutsu history list`.
        #[arg(long)]
        session: Option<String>,
        #[command(flatten)]
        agent: AgentArgs,
        /// Where to write the revised plan.
        #[arg(long)]
        out: std::path::PathBuf,
    },
    /// Ask a second agent to challenge a plan. It cannot change it.
    Review {
        /// The plan to review.
        file: std::path::PathBuf,
        /// The task, if the plan's title does not say it well enough.
        #[arg(long)]
        task: Option<String>,
        #[command(flatten)]
        agent: AgentArgs,
        /// Write the plan with the review recorded in its provenance.
        #[arg(long)]
        record: Option<std::path::PathBuf>,
    },
    /// Check a plan file against the schema and its structure, and show the
    /// order its steps would run in.
    Check {
        /// The plan file.
        file: std::path::PathBuf,
        /// Treat it as a stored plan, which may carry KeyJutsu's own state.
        /// By default it is checked as an agent's proposal, which may not.
        #[arg(long)]
        stored: bool,
    },
    /// Validate a plan against this machine: syntax, commands, parameters,
    /// tools, preconditions, privilege, risk and, where trustworthy, a
    /// -WhatIf dry run. Nothing the plan names is run.
    Validate {
        /// The plan file.
        file: std::path::PathBuf,
        /// Skip -WhatIf dry runs.
        #[arg(long)]
        no_dry_run: bool,
        /// Print the full report as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Print each step's hash: what an approval of that step binds to.
    Hash {
        /// The plan file.
        file: std::path::PathBuf,
    },
    /// Approve a plan and seal it into an immutable snapshot. Every step is
    /// approved except critical ones, which each need --confirm with their
    /// typed phrase.
    Approve {
        /// The plan file.
        file: std::path::PathBuf,
        /// Where to write the snapshot.
        #[arg(long)]
        out: std::path::PathBuf,
        /// Approve a critical step: STEP="TYPED PHRASE".
        #[arg(long = "confirm", value_name = "STEP=PHRASE")]
        confirmations: Vec<String>,
        /// Replace an existing file at --out.
        #[arg(long)]
        force: bool,
        /// Skip -WhatIf dry runs during the validation that precedes sealing.
        #[arg(long)]
        no_dry_run: bool,
    },
    /// Download every artifact the plan needs, check it against its pinned
    /// hash, and keep it for the run. Nothing is downloaded while a plan runs.
    Stage {
        /// The plan file.
        file: std::path::PathBuf,
        /// Write a copy of the plan with the hash of each unpinned artifact
        /// filled in, for you to review before approving.
        #[arg(long, value_name = "FILE")]
        pin: Option<std::path::PathBuf>,
    },
    /// Check a sealed snapshot has not been altered.
    Verify {
        /// The approved snapshot.
        snapshot: std::path::PathBuf,
        /// Also compare this machine with the one the plan was approved on.
        #[arg(long)]
        environment: bool,
    },
    /// Show what changed between two versions of a plan, and what that affects.
    Diff {
        /// The earlier plan or snapshot.
        old: std::path::PathBuf,
        /// The later one.
        new: std::path::PathBuf,
    },
}

#[derive(clap::Args)]
struct ShellArgs {
    /// PowerShell 7 if it is installed, otherwise Windows PowerShell, which
    /// every Windows has.
    #[arg(long, value_enum)]
    shell: Option<ShellChoice>,
    /// Skip your shell profile, history predictions and history saving.
    #[arg(long)]
    clean: bool,
}

#[derive(clap::Args)]
struct PerformanceArgs {
    /// Performance: each key you press types the next character. Assisted:
    /// each key types a short burst, up to the end of a word. Auto: it types
    /// and runs everything by itself. Direct: no typing effect at all.
    #[arg(long, value_enum, default_value_t = ModeChoice::Performance)]
    mode: ModeChoice,
    /// In performance mode, advance a word per key instead of a character.
    #[arg(long)]
    turbo: bool,
    /// What submits a finished command: any key, only Enter, or KeyJutsu
    /// itself once it is typed.
    #[arg(long, value_enum, default_value_t = SubmitChoice::AnyKey)]
    submit: SubmitChoice,
}

#[derive(Clone, Copy, ValueEnum)]
enum ShellChoice {
    Pwsh,
    Powershell,
    Cmd,
}

#[derive(Clone, Copy, ValueEnum)]
enum PresentationChoice {
    /// KeyJutsu's questions and step titles appear on screen.
    Standard,
    /// Nothing of KeyJutsu's appears in the console: step titles stay out of
    /// the window title, and a question is put in the title bar, with the
    /// answer typed unseen.
    Discreet,
}

#[derive(Clone, Copy, ValueEnum)]
enum ModeChoice {
    Performance,
    Assisted,
    Auto,
    Direct,
}

#[derive(Clone, Copy, ValueEnum)]
enum SubmitChoice {
    AnyKey,
    Enter,
    Auto,
}

impl ShellArgs {
    fn options(&self) -> SessionOptions {
        let mut options = SessionOptions::new(match self.shell {
            Some(ShellChoice::Pwsh) => ShellKind::Pwsh,
            Some(ShellChoice::Powershell) => ShellKind::WindowsPowershell,
            Some(ShellChoice::Cmd) => ShellKind::Cmd,
            None if keyjutsu_core::terminal::shell::locate(ShellKind::Pwsh).is_some() => ShellKind::Pwsh,
            None => ShellKind::WindowsPowershell,
        });
        if self.clean {
            options.profile = ProfileMode::Clean;
        }
        options
    }
}

impl PerformanceArgs {
    fn config(&self) -> PerformanceConfig {
        PerformanceConfig {
            mode: match self.mode {
                ModeChoice::Performance => ExecutionMode::Performance,
                ModeChoice::Assisted => ExecutionMode::Assisted,
                ModeChoice::Auto => ExecutionMode::AutoPerformance,
                ModeChoice::Direct => ExecutionMode::Direct,
            },
            advance: if self.turbo { AdvanceStyle::Turbo } else { AdvanceStyle::Pure },
            submit: match self.submit {
                SubmitChoice::AnyKey => SubmitPolicy::AnyKey,
                SubmitChoice::Enter => SubmitPolicy::RequireEnter,
                SubmitChoice::Auto => SubmitPolicy::AutoSubmit,
            },
            ..PerformanceConfig::default()
        }
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Command::Doctor { json } => doctor(json),
        Command::Diagnostics(DiagnosticsCommand::Preview) => {
            print!("{}", keyjutsu_core::diagnostics::collect());
            ExitCode::SUCCESS
        }
        Command::Diagnostics(DiagnosticsCommand::Save { file, force }) => save_diagnostics(&file, force),
        Command::Agents { action: None, json } => agent_cli::list(json),
        Command::Agents { action: Some(AgentsCommand::Check { live }), .. } => agent_cli::check(live),
        Command::Plan(PlanCommand::Propose { task, agent, files, folder, out }) => {
            agent_cli::propose(&task, &agent.agent, agent.send, &files, folder.as_deref(), &out)
        }
        Command::Plan(PlanCommand::Revise { file, step, guidance, task, session, agent, out }) => {
            agent_cli::revise(agent_cli::Revise {
                file: &file,
                step: &step,
                guidance: &guidance,
                task: task.as_deref(),
                session: session.as_deref(),
                agent: &agent.agent,
                send: agent.send,
                out: &out,
            })
        }
        Command::Plan(PlanCommand::Review { file, task, agent, record }) => {
            agent_cli::review(&file, task.as_deref(), &agent.agent, agent.send, record.as_deref())
        }
        Command::Recover { snapshot, from, steps, confirm, clean } => {
            recover_cli::run(recover_cli::RecoverArgs {
                snapshot: &snapshot,
                from,
                steps: &steps,
                confirm,
                clean,
            })
        }
        Command::Git(GitCommand::Diff { snapshot }) => git_cli::diff(&snapshot),
        Command::Setup(SetupCommand::Path { action }) => setup_cli::path(action.as_str()),
        Command::Setup(SetupCommand::Explorer { action }) => setup_cli::explorer(action.as_str()),
        Command::History(HistoryCommand::List) => history_cli::history_list(),
        Command::History(HistoryCommand::Show { id }) => history_cli::history_show(&id),
        Command::History(HistoryCommand::Recheck { id }) => history_cli::history_recheck(&id),
        Command::Technique(TechniqueCommand::Promote { session, name, description, params }) => {
            history_cli::technique_promote(&session, &name, &description, &params)
        }
        Command::Technique(TechniqueCommand::List) => history_cli::technique_list(),
        Command::Technique(TechniqueCommand::Use { id, params, out }) => {
            history_cli::technique_use(&id, &params, &out)
        }
        Command::Technique(TechniqueCommand::Revise { id, template }) => {
            history_cli::technique_revise(&id, &template)
        }
        Command::Technique(TechniqueCommand::Export { id, out }) => history_cli::technique_export(&id, &out),
        Command::Technique(TechniqueCommand::Import { file }) => history_cli::technique_import(&file),
        Command::Store(StoreCommand::Clear { history, techniques, artifacts }) => {
            history_cli::store_clear(history, techniques, artifacts)
        }
        Command::Run { snapshot, mode, clean, resume, settle, isolate, ephemeral, presentation } => {
            run_cli::run(run_cli::RunArgs {
                snapshot: &snapshot,
                mode: mode.map(|m| match m {
                    ModeChoice::Performance => ExecutionMode::Performance,
                    ModeChoice::Assisted => ExecutionMode::Assisted,
                    ModeChoice::Auto => ExecutionMode::AutoPerformance,
                    ModeChoice::Direct => ExecutionMode::Direct,
                }),
                clean,
                resume: resume.map(|from| from.unwrap_or_else(|| run_cli::checkpoint_path(&snapshot))),
                settle: &settle,
                isolate: isolate.map(|i| match i {
                    IsolateChoice::Worktree => run_cli::Isolation::Worktree,
                    IsolateChoice::Branch => run_cli::Isolation::Branch,
                }),
                ephemeral,
                discreet: matches!(presentation, PresentationChoice::Discreet),
            })
        }
        Command::Shell(shell) => session(shell.options(), None),
        Command::Demo { shell, performance } => {
            let options = shell.options();
            let script = demo::safe_demo(options.shell);
            session(options, Some(console::Performance { script, config: performance.config() }))
        }
        Command::Plan(PlanCommand::Check { file, stored }) => plan_check(&file, stored),
        Command::Plan(PlanCommand::Hash { file }) => plans::hash(&file),
        Command::Plan(PlanCommand::Validate { file, no_dry_run, json }) => {
            plans::validate(&file, !no_dry_run, json)
        }
        Command::Plan(PlanCommand::Approve { file, out, confirmations, force, no_dry_run }) => {
            plans::approve(&file, &out, &confirmations, force, !no_dry_run)
        }
        Command::Plan(PlanCommand::Stage { file, pin }) => plans::stage(&file, pin.as_deref()),
        Command::Plan(PlanCommand::Verify { snapshot, environment }) => plans::verify(&snapshot, environment),
        Command::Plan(PlanCommand::Diff { old, new }) => plans::diff(&old, &new),
        Command::Perform { commands, shell, performance } => {
            let script = StagedScript {
                steps: commands
                    .iter()
                    .enumerate()
                    .map(|(i, command)| StagedStep {
                        id: format!("step-{}", i + 1),
                        title: format!("Step {}", i + 1),
                        command: command.clone(),
                        mode: None,
                        submit: None,
                        answers: None,
                    })
                    .collect(),
            };
            let config = performance.config();
            if let Err(e) = script.validate(config.mode) {
                eprintln!("keyjutsu: {e}");
                return ExitCode::from(2);
            }
            session(shell.options(), Some(console::Performance { script, config }))
        }
    }
}

fn session(options: SessionOptions, performance: Option<console::Performance>) -> ExitCode {
    let titles: Vec<String> = performance
        .as_ref()
        .map(|p| p.script.steps.iter().map(|s| s.title.clone()).collect())
        .unwrap_or_default();
    let summary = match console::run(options, performance, None, None) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("keyjutsu: {e}");
            return ExitCode::FAILURE;
        }
    };
    if let Some(Err(e)) = &summary.armed {
        eprintln!("keyjutsu: could not arm: {e}");
        return ExitCode::FAILURE;
    }
    let mut failed = false;
    for (index, outcome) in &summary.outcomes {
        let title = titles.get(*index).map(String::as_str).unwrap_or("step");
        let text = match outcome {
            StepOutcome::Succeeded { exit_code } => format!("succeeded (exit {exit_code})"),
            StepOutcome::Unverified => {
                "finished; success unverified (this shell reports no exit codes)".into()
            }
            StepOutcome::Failed { exit_code } => {
                failed = true;
                format!("FAILED (exit {exit_code})")
            }
        };
        println!("{:>2}. {title}: {text}", index + 1);
    }
    if !titles.is_empty() && summary.outcomes.len() < titles.len() && !failed {
        println!(
            "{} of {} ran before the performance ended.",
            summary.outcomes.len(),
            count(titles.len(), "step", "steps")
        );
    }
    if failed { ExitCode::FAILURE } else { ExitCode::SUCCESS }
}

fn save_diagnostics(file: &std::path::Path, force: bool) -> ExitCode {
    if file.exists() && !force {
        eprintln!("keyjutsu: {} already exists; pass --force to replace it", file.display());
        return ExitCode::FAILURE;
    }
    match std::fs::write(file, keyjutsu_core::diagnostics::collect()) {
        Ok(()) => {
            println!("Saved the diagnostic bundle to {}.", file.display());
            println!("It is plain text: read it before you send it to anyone. Nothing has been sent.");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("keyjutsu: could not write {}: {e}", file.display());
            ExitCode::FAILURE
        }
    }
}

fn doctor(json: bool) -> ExitCode {
    let report = readiness::scan();
    if json {
        match serde_json::to_string_pretty(&report) {
            Ok(text) => println!("{text}"),
            Err(e) => {
                eprintln!("keyjutsu: {e}");
                return ExitCode::FAILURE;
            }
        }
    } else {
        println!("KeyJutsu {}", report.keyjutsu_version);
        println!();
        for check in &report.checks {
            let mark = match check.status {
                CheckStatus::Ok => "ok  ",
                CheckStatus::Warning => "warn",
                CheckStatus::Unavailable => "FAIL",
            };
            println!("  [{mark}] {:<32} {}", check.name, check.detail);
        }
        println!();
        for shell in &report.shells {
            println!(
                "  {:<24} {}  {}",
                shell.kind.display_name(),
                shell.version.as_deref().unwrap_or("version unknown"),
                shell.path
            );
        }
        let p = &report.terminal_profile;
        println!();
        println!(
            "  Terminal profile: {} ({}), {} {}pt, scheme {}",
            p.name, p.source, p.font_face, p.font_size, p.color_scheme.name
        );
    }
    let blocked = report.checks.iter().any(|c| c.status == CheckStatus::Unavailable);
    if blocked { ExitCode::FAILURE } else { ExitCode::SUCCESS }
}

fn plan_check(file: &std::path::Path, stored: bool) -> ExitCode {
    use keyjutsu_core::plan::{PlanError, parse_plan, parse_proposal};
    let text = match std::fs::read_to_string(file) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("keyjutsu: cannot read {}: {e}", file.display());
            return ExitCode::from(2);
        }
    };
    let result = if stored { parse_plan(&text) } else { parse_proposal(&text) };
    match result {
        Ok(valid) => {
            let plan = valid.plan();
            println!(
                "{}: valid ({})",
                plan.title.as_deref().unwrap_or(&plan.plan_id),
                count(plan.steps.len(), "step", "steps")
            );
            println!();
            for (n, id) in valid.graph().topological_order().enumerate() {
                let branch = plan.edges.iter().filter(|e| e.to == id && e.when.is_some()).count();
                let note = if branch > 0 { "  (conditional)" } else { "" };
                let title = plan.step(id).map(|s| s.title.as_str()).unwrap_or(id);
                println!("  {:>2}. {id}: {title}{note}", n + 1);
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{}: {e}", file.display());
            match &e {
                PlanError::Schema { violations } => {
                    for v in violations {
                        let at = if v.at.is_empty() { "(document)" } else { v.at.as_str() };
                        eprintln!("  {at}: {}", v.message);
                    }
                }
                PlanError::Invalid { problems } => {
                    for p in problems {
                        eprintln!("  {p}");
                    }
                }
                _ => {}
            }
            ExitCode::FAILURE
        }
    }
}
