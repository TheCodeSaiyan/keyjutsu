//! `keyjutsu`: the command-line front end.
//!
//! Everything here is presentation and argument parsing. Sessions, staged
//! input and the rules about what reaches a shell come from keyjutsu-core,
//! the same code the desktop app uses.

mod console;

use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};
use keyjutsu_core::execution::{
    AdvanceStyle, ExecutionMode, PerformanceConfig, StagedScript, StagedStep, StepOutcome, SubmitPolicy,
};
use keyjutsu_core::readiness::{self, CheckStatus};
use keyjutsu_core::terminal::{ProfileMode, ShellKind};
use keyjutsu_core::{SessionOptions, demo};

#[derive(Parser)]
#[command(name = "keyjutsu", version, about = "Validated commands, theatrically typed.", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Check this machine is ready: Windows, ConPTY, shells and staged input.
    Doctor {
        /// Print the full report as JSON.
        #[arg(long)]
        json: bool,
    },
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
    /// Perform commands you supply. Before approved plans exist (Milestone 8)
    /// this is how to stage your own commands; they carry no approval and run
    /// exactly as if you had typed them.
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
enum PlanCommand {
    /// Check a plan file against the schema and its structure, and show the
    /// order its steps would run in.
    Check {
        file: std::path::PathBuf,
        /// Treat it as a stored plan, which may carry KeyJutsu's own state.
        /// By default it is checked as an agent's proposal, which may not.
        #[arg(long)]
        stored: bool,
    },
}

#[derive(clap::Args)]
struct ShellArgs {
    #[arg(long, value_enum, default_value_t = ShellChoice::Pwsh)]
    shell: ShellChoice,
    /// Skip your shell profile, history predictions and history saving.
    #[arg(long)]
    clean: bool,
}

#[derive(clap::Args)]
struct PerformanceArgs {
    #[arg(long, value_enum, default_value_t = ModeChoice::Performance)]
    mode: ModeChoice,
    /// In performance mode, advance a word per key instead of a character.
    #[arg(long)]
    turbo: bool,
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
            ShellChoice::Pwsh => ShellKind::Pwsh,
            ShellChoice::Powershell => ShellKind::WindowsPowershell,
            ShellChoice::Cmd => ShellKind::Cmd,
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
        Command::Shell(shell) => session(shell.options(), None),
        Command::Demo { shell, performance } => {
            let options = shell.options();
            let script = demo::safe_demo(options.shell);
            session(options, Some(console::Performance { script, config: performance.config() }))
        }
        Command::Plan(PlanCommand::Check { file, stored }) => plan_check(&file, stored),
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
    let summary = match console::run(options, performance) {
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
        println!("{} of {} steps ran before the performance ended.", summary.outcomes.len(), titles.len());
    }
    if failed { ExitCode::FAILURE } else { ExitCode::SUCCESS }
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
                CheckStatus::NotYetBuilt => "--  ",
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
                "{}: valid ({} steps)",
                plan.title.as_deref().unwrap_or(&plan.plan_id),
                plan.steps.len()
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
