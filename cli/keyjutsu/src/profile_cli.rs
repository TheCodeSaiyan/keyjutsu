//! `keyjutsu profile`: the work profiles a plan can be made and checked
//! under, and which one applies here.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use keyjutsu_core::store::{Store, default_root};
use keyjutsu_core::validation::profile::Profile;
use keyjutsu_core::work_profile::{self, Chosen, Decision, Places, Why};

/// What the profile is wanted for. Making a plan needs one: with nothing set,
/// KeyJutsu asks first. Checking a plan file that already exists goes ahead
/// without one, as it did before profiles, and says so.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    Planning,
    Checking,
}

fn start(folder: Option<&Path>) -> PathBuf {
    folder.map(Path::to_owned).unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| ".".into()))
}

fn store() -> Option<Store> {
    Store::open(&default_root()).ok()
}

/// The profile that applies to a command run in `folder` (or here), or the
/// one named with `--profile`, which overrides it and is said to.
pub fn chosen(
    name: Option<&str>,
    folder: Option<&Path>,
    purpose: Purpose,
) -> Result<Option<Profile>, ExitCode> {
    applied(name, folder, purpose).map(|o| o.map(|(p, _)| p))
}

/// As [`chosen`], with why the profile applies, for recording with the plan.
pub fn applied(
    name: Option<&str>,
    folder: Option<&Path>,
    purpose: Purpose,
) -> Result<Option<(Profile, String)>, ExitCode> {
    let places = Places::default();
    let store = store();
    let decision = work_profile::decide(&places, store.as_ref(), &start(folder));
    let fail = |message: String| {
        eprintln!("keyjutsu: {message}");
        Err(ExitCode::from(2))
    };
    if let Some(name) = name {
        return match work_profile::hand_pick(&places, &decision, name) {
            Ok(c) => {
                note(&c);
                Ok(Some((c.profile, describe(&c.why))))
            }
            Err(e) => fail(format!("{e}; `keyjutsu profile list` shows them")),
        };
    }
    match decision {
        Decision::Chosen(c) => {
            note(&c);
            Ok(Some((c.profile, describe(&c.why))))
        }
        Decision::NeedsConfirmation(u) => fail(format!(
            "{} sets the work profile {}, which is less strict than this machine's {}. \
             `keyjutsu profile confirm` accepts it for these files; --profile NAME chooses one by hand.",
            u.dir,
            u.folder_profile.name(),
            u.machine_profile
        )),
        Decision::Broken { reason, .. } => {
            fail(format!("no work profile can be chosen here: {reason}. --profile NAME chooses one by hand."))
        }
        Decision::NoDefault { .. } if purpose == Purpose::Planning => fail(
            "no work profile is set for this machine. Choose the default with `keyjutsu profile default NAME` \
             (`keyjutsu profile list` shows them), or pass --profile NAME."
                .into(),
        ),
        Decision::NoDefault { .. } => {
            eprintln!("No work profile is set, so none is applied. `keyjutsu profile default NAME` sets one.");
            Ok(None)
        }
    }
}

/// One line on stderr saying which profile applies and why, so `--json`
/// output stays clean.
fn note(c: &Chosen) {
    eprintln!("Work profile: {} ({}), {}.", c.profile.title(), c.profile.name(), describe(&c.why));
}

fn describe(why: &Why) -> String {
    match why {
        Why::Folder { dir, .. } => format!("set by {dir}"),
        Why::ConfirmedFolder { dir, .. } => format!("set by {dir}, which you confirmed"),
        Why::Machine => "this machine's default".into(),
        Why::HandPicked { instead_of: Some(other), reason } => format!("{reason}, instead of {other}"),
        Why::HandPicked { instead_of: None, reason } => reason.clone(),
    }
}

pub fn list(folder: Option<&Path>) -> ExitCode {
    let catalogue = match work_profile::catalogue(&Places::default(), &start(folder)) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("keyjutsu: {e}");
            return ExitCode::FAILURE;
        }
    };
    for (name, title) in catalogue.names() {
        let Ok(p) = catalogue.resolve(&name) else {
            println!("{name:<18} {title} (does not resolve)");
            continue;
        };
        let parent = p.chain.get(1).map(|l| format!(", extends {}", l.name)).unwrap_or_default();
        println!("{name:<18} {title}{parent}");
    }
    ExitCode::SUCCESS
}

pub fn show(name: &str) -> ExitCode {
    let places = Places::default();
    let resolved = work_profile::catalogue(&places, &start(None))
        .and_then(|c| c.resolve(name).map_err(|e| e.to_string()));
    let profile = match resolved {
        Ok(p) => p,
        Err(e) => {
            eprintln!("keyjutsu: {e}; `keyjutsu profile list` shows them");
            return ExitCode::from(2);
        }
    };
    let chain: Vec<String> = profile.chain.iter().map(|l| format!("{} ({})", l.name, l.origin)).collect();
    println!("{} ({})", profile.title(), profile.name());
    if let Some(s) = &profile.chain[0].summary {
        println!("{s}");
    }
    println!("Chain: {}", chain.join(" → "));
    println!();
    println!("What validation enforces:");
    match profile.require_recovery_from {
        Some(r) => println!("  a recovery for every step at {} risk or above", level(r)),
        None => println!("  no recovery required"),
    }
    if let Some(m) = profile.max_risk {
        println!("  nothing above {} risk", level(m));
    }
    if profile.block_opaque_code {
        println!("  no code KeyJutsu cannot see into");
    }
    match profile.confirm_each_from {
        Some(r) if r < keyjutsu_core::plan::model::RiskLevel::Critical => {
            println!("  its own typed confirmation for every step at {} risk or above", level(r))
        }
        _ => {}
    }
    for r in &profile.raise {
        println!(
            "  at least {} risk: {} ({}, from {})",
            level(r.rule.to),
            r.rule.commands.join(", "),
            r.rule.reason,
            r.profile
        );
    }
    for d in &profile.deny {
        println!("  refused: {} ({}, from {})", d.rule.commands.join(", "), d.rule.reason, d.profile);
    }
    println!();
    println!("What the agents are told, after their ground rules:");
    for line in profile.agent_rules().lines() {
        println!("  {line}");
    }
    ExitCode::SUCCESS
}

/// Show this machine's default, or set it.
pub fn default(name: Option<&str>) -> ExitCode {
    let places = Places::default();
    match name {
        Some(name) => match work_profile::set_machine_default(&places, name) {
            Ok(()) => {
                println!("This machine's default work profile is now {name}.");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("keyjutsu: {e}");
                ExitCode::from(2)
            }
        },
        None => match work_profile::machine_default(&places) {
            Ok(Some(d)) => {
                println!("{d}");
                ExitCode::SUCCESS
            }
            Ok(None) => {
                println!("No default is set. `keyjutsu profile default NAME` sets one.");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("keyjutsu: {e}");
                ExitCode::FAILURE
            }
        },
    }
}

/// Which profile applies in `folder` (or here), and why.
pub fn which(folder: Option<&Path>) -> ExitCode {
    let places = Places::default();
    let store = store();
    let decision = work_profile::decide(&places, store.as_ref(), &start(folder));
    let start = match &decision {
        Decision::Chosen(c) => &c.start,
        Decision::NeedsConfirmation(u) => &u.start,
        Decision::Broken { start, .. } | Decision::NoDefault { start } => start,
    };
    println!("Started in {}", start.typed.display());
    match &start.real {
        Some(real) if Path::new(real) != start.typed => println!("which is really {real}"),
        _ => {}
    }
    match &decision {
        Decision::Chosen(c) => {
            println!("Work profile: {} ({}), {}.", c.profile.title(), c.profile.name(), describe(&c.why));
            if !c.leftovers.is_empty() {
                println!("Ignored in .keyjutsu: {}", c.leftovers.join(", "));
            }
            ExitCode::SUCCESS
        }
        Decision::NeedsConfirmation(u) => {
            println!(
                "{} sets {} ({}), which is less strict than this machine's {}. \
                 It applies once you confirm it with `keyjutsu profile confirm`.",
                u.dir,
                u.folder_profile.title(),
                u.folder_profile.name(),
                u.machine_profile
            );
            ExitCode::SUCCESS
        }
        Decision::Broken { reason, .. } => {
            println!("No work profile can be chosen here: {reason}.");
            println!("Planning stops until you choose one by hand with --profile NAME.");
            ExitCode::FAILURE
        }
        Decision::NoDefault { .. } => {
            println!("No work profile is set. `keyjutsu profile default NAME` sets this machine's.");
            ExitCode::SUCCESS
        }
    }
}

/// Accept a folder's less strict default for the files as they are now.
pub fn confirm(folder: Option<&Path>) -> ExitCode {
    let places = Places::default();
    let Some(store) = store() else {
        eprintln!("keyjutsu: the encrypted store cannot be opened to keep the confirmation");
        return ExitCode::FAILURE;
    };
    match work_profile::decide(&places, Some(&store), &start(folder)) {
        Decision::NeedsConfirmation(u) => {
            match work_profile::confirm(&store, &u, &keyjutsu_core::fingerprint::now_rfc3339()) {
                Ok(c) => {
                    println!(
                        "Confirmed: plans made in {} use {} ({}) rather than this machine's {}, \
                         until its .keyjutsu files change.",
                        u.dir,
                        c.profile.title(),
                        c.profile.name(),
                        u.machine_profile
                    );
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("keyjutsu: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        Decision::Chosen(c) => {
            println!(
                "Nothing to confirm: {} ({}) applies, {}.",
                c.profile.title(),
                c.profile.name(),
                describe(&c.why)
            );
            ExitCode::SUCCESS
        }
        Decision::Broken { reason, .. } => {
            eprintln!("keyjutsu: nothing can be confirmed here: {reason}");
            ExitCode::FAILURE
        }
        Decision::NoDefault { .. } => {
            println!("Nothing to confirm: no folder or machine default is set.");
            ExitCode::SUCCESS
        }
    }
}

fn level(r: keyjutsu_core::plan::model::RiskLevel) -> String {
    format!("{r:?}").to_ascii_lowercase()
}
