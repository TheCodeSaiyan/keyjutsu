//! `keyjutsu profile`: the work profiles a plan can be checked under.

use std::process::ExitCode;

use keyjutsu_core::validation::profile::{Catalogue, Profile};

/// The profile named on the command line, if one was.
pub fn chosen(name: Option<&str>) -> Result<Option<Profile>, ExitCode> {
    let Some(name) = name else { return Ok(None) };
    match Catalogue::built_in().resolve(name) {
        Ok(p) => Ok(Some(p)),
        Err(e) => {
            eprintln!("keyjutsu: {e}; `keyjutsu profile list` shows them");
            Err(ExitCode::from(2))
        }
    }
}

pub fn list() -> ExitCode {
    let catalogue = Catalogue::built_in();
    for (name, title) in catalogue.names() {
        let Ok(p) = catalogue.resolve(&name) else { continue };
        let parent = p.chain.get(1).map(|l| format!(", extends {}", l.name)).unwrap_or_default();
        println!("{name:<18} {title}{parent}");
    }
    ExitCode::SUCCESS
}

pub fn show(name: &str) -> ExitCode {
    let profile = match chosen(Some(name)) {
        Ok(Some(p)) => p,
        Ok(None) => return ExitCode::from(2),
        Err(c) => return c,
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

fn level(r: keyjutsu_core::plan::model::RiskLevel) -> String {
    format!("{r:?}").to_ascii_lowercase()
}
