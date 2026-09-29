//! Validation against real shells on this machine. Every plan here uses only
//! what Windows itself provides, so the suite means the same on any Windows
//! 11 machine, CI included.

#![cfg(windows)]
#![allow(clippy::unwrap_used)] // Helpers outside #[test] functions may unwrap too.

use std::path::{Path, PathBuf};

use keyjutsu_plan::model::{EvidenceResult, ProofLevel, Readiness, RiskLevel, StepState};
use keyjutsu_plan::{ValidPlan, parse_plan};
use keyjutsu_validation::{Options, Report, validate};
use serde_json::{Value, json};

fn plan(steps: Value) -> ValidPlan {
    plan_with(steps, json!([]), json!([]))
}

fn plan_with(steps: Value, requirements: Value, assumptions: Value) -> ValidPlan {
    let v = json!({
        "schema_version": "1.0", "plan_id": "p", "task_id": "t",
        "target": {"id": "local", "kind": "local_windows"},
        "agent": {"name": "codex", "version": "1"},
        "requirements": requirements,
        "environment_assumptions": assumptions,
        "steps": steps
    });
    parse_plan(&v.to_string()).unwrap_or_else(|e| panic!("{e:?}"))
}

fn step(id: &str, shell: &str, command: &str) -> Value {
    json!({"id": id, "title": id, "objective": "Test.", "kind": "command", "shell": {"kind": shell}, "commands": [{"text": command}]})
}

fn state<'a>(r: &'a Report, id: &str) -> &'a StepState {
    r.steps.get(id).unwrap_or_else(|| panic!("no state for {id}"))
}

fn failed(s: &StepState, check: &str) -> Vec<String> {
    s.evidence
        .iter()
        .filter(|e| e.check == check && e.result == EvidenceResult::Failed)
        .map(|e| e.detail.clone().unwrap_or_default())
        .collect()
}

/// A scratch path under the build directory, unique to one test.
fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("validation").join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn forward(p: &Path) -> String {
    p.display().to_string().replace('\\', "/")
}

#[test]
fn a_read_only_step_is_ready_with_high_proof() {
    let r = validate(
        &plan(json!([step("svc", "pwsh", "Get-Service -Name Winmgmt | Format-Table Status")])),
        Options::default(),
    );
    let s = state(&r, "svc");
    assert_eq!(s.readiness, Readiness::Ready, "{s:#?}");
    assert_eq!(s.proof_level, ProofLevel::High);
    assert_eq!(s.assessed_risk, Some(RiskLevel::Low));
    assert!(s.evidence.iter().any(|e| e.check == "parameters" && e.result == EvidenceResult::Passed));
    assert!(r.problems.is_empty(), "{:?}", r.problems);
}

#[test]
fn a_mistyped_parameter_or_broken_syntax_is_invalid() {
    let r = validate(
        &plan(json!([
            step("typo", "pwsh", "Get-Service -Nmae Winmgmt"),
            step("ambiguous", "pwsh", "Get-ChildItem -P C:/"),
            step("syntax", "pwsh", "Write-Output ('a' +")
        ])),
        Options::default(),
    );
    assert_eq!(state(&r, "typo").readiness, Readiness::Invalid);
    assert!(failed(state(&r, "typo"), "parameters")[0].contains("-Nmae"));
    assert_eq!(state(&r, "ambiguous").readiness, Readiness::Invalid);
    assert!(failed(state(&r, "ambiguous"), "parameters")[0].contains("ambiguous"));
    assert_eq!(state(&r, "syntax").readiness, Readiness::Invalid);
    assert_eq!(state(&r, "syntax").proof_level, ProofLevel::Low);
}

#[test]
fn a_missing_command_or_tool_blocks_the_step() {
    let steps = json!([
        step("cmd-missing", "pwsh", "Get-KeyJutsuNothingHere"),
        {
            "id": "tool", "title": "tool", "objective": "Test.", "kind": "command", "shell": {"kind": "pwsh"},
            "commands": [{"text": "Get-Date"}],
            "tool_requirements": [
                {"name": "missing", "executable": "keyjutsu-no-such-tool"},
                {"name": "cmd", "executable": "cmd.exe", "version": ">=10"}
            ]
        }
    ]);
    let r = validate(&plan(steps), Options::default());
    assert_eq!(state(&r, "cmd-missing").readiness, Readiness::Blocked);
    let tool = state(&r, "tool");
    assert_eq!(tool.readiness, Readiness::Blocked);
    assert!(failed(tool, "tools")[0].contains("keyjutsu-no-such-tool"));
    assert!(
        tool.evidence.iter().any(|e| e.check == "tools"
            && e.result == EvidenceResult::Passed
            && e.detail.as_deref().is_some_and(|d| d.contains("cmd.exe") && d.contains("version 10."))),
        "cmd.exe's version is read from its file: {tool:#?}"
    );
}

#[test]
fn validating_a_destructive_step_leaves_the_machine_alone() {
    let dir = scratch("destructive");
    std::fs::write(dir.join("keep.txt"), "still here").unwrap();
    let command = format!("Remove-Item -Recurse -Force -LiteralPath {}", forward(&dir));
    let mut s = step("wipe", "pwsh", &command);
    s["proposed_risk"] = json!({"level": "low", "rationale": "Just tidying."});
    let r = validate(&plan(json!([s])), Options::default());

    assert!(dir.join("keep.txt").exists(), "validation deleted the directory");
    let st = state(&r, "wipe");
    assert_eq!(st.assessed_risk, Some(RiskLevel::Critical));
    assert_eq!(st.readiness, Readiness::NeedsReview, "the agent under-stated the risk: {st:#?}");
    let dry = st.evidence.iter().find(|e| e.check == "dry run").expect("a dry run");
    assert_eq!(dry.result, EvidenceResult::Passed, "{dry:?}");
    assert!(dry.detail.as_deref().unwrap().contains("What if:"), "{dry:?}");
    assert_eq!(st.proof_level, ProofLevel::High, "a clean dry run of every line");
}

/// A wildcard is judged by what it could match, not by what it matches
/// today, and before approval the dry run shows every file it matches now.
/// A pipeline that feeds a delete is judged the same way.
#[test]
fn a_wildcard_is_rated_critical_and_shown_expanded_before_approval() {
    let dir = scratch("wildcard");
    for f in ["a.log", "b.log", "keep.txt"] {
        std::fs::write(dir.join(f), "x").unwrap();
    }
    let honest = |mut s: Value| {
        s["proposed_risk"] = json!({"level": "critical", "rationale": "Deletes the logs."});
        s
    };
    let wildcard = honest(step("logs", "pwsh", &format!("Remove-Item -Path {}/*.log", forward(&dir))));
    let piped = honest(step(
        "piped",
        "pwsh",
        &format!("Get-ChildItem -LiteralPath {} -Filter *.log | Remove-Item", forward(&dir)),
    ));
    let r = validate(&plan(json!([wildcard, piped])), Options::default());
    for f in ["a.log", "b.log", "keep.txt"] {
        assert!(dir.join(f).exists(), "validation deleted {f}");
    }

    let st = state(&r, "logs");
    assert_eq!(st.assessed_risk, Some(RiskLevel::Critical), "{st:#?}");
    let dry = st.evidence.iter().find(|e| e.check == "dry run").expect("a dry run");
    let shown = dry.detail.as_deref().unwrap_or_default();
    assert!(shown.contains("a.log") && shown.contains("b.log"), "every match is shown: {shown}");
    assert!(!shown.contains("keep.txt"), "{shown}");

    let st = state(&r, "piped");
    assert_eq!(st.assessed_risk, Some(RiskLevel::Critical), "{st:#?}");
    let why = st.risk_reasons.join(" | ");
    assert!(why.contains("pipeline"), "{why}");
}

#[test]
fn a_trailing_comment_cannot_turn_a_dry_run_into_a_real_one() {
    let dir = scratch("comment");
    let file = dir.join("victim.txt");
    std::fs::write(&file, "still here").unwrap();
    // Appending " -WhatIf" to this line would put it inside the comment.
    let command = format!("Remove-Item -LiteralPath {} # tidy up", forward(&file));
    let r = validate(&plan(json!([step("rm", "pwsh", &command)])), Options::default());
    assert!(file.exists(), "the dry run really deleted the file");
    let st = state(&r, "rm");
    assert!(
        st.evidence.iter().any(|e| e.check == "dry run" && e.result == EvidenceResult::Passed),
        "{st:#?}"
    );
}

#[test]
fn expressions_are_never_evaluated_by_a_dry_run() {
    let dir = scratch("expression");
    std::fs::write(dir.join("x.txt"), "x").unwrap();
    let command = format!("Remove-Item -LiteralPath (Join-Path '{}' 'x.txt')", forward(&dir));
    let r = validate(&plan(json!([step("rm", "pwsh", &command)])), Options::default());
    let st = state(&r, "rm");
    let skipped = st.evidence.iter().find(|e| e.check == "dry run").unwrap();
    assert_eq!(skipped.result, EvidenceResult::NotApplicable);
    // The nested Join-Path is a second command inside the line, which rules
    // out a dry run before its being an expression even comes up.
    let why = skipped.detail.as_deref().unwrap();
    assert!(why.contains("more than one command") || why.contains("expressions"), "{why}");
    assert!(dir.join("x.txt").exists());
    assert_eq!(st.proof_level, ProofLevel::Medium);
}

#[test]
fn a_variable_in_the_arguments_rules_out_a_dry_run() {
    let r = validate(
        &plan(json!([step("rm", "pwsh", "Remove-Item -LiteralPath $env:TEMP/keyjutsu-nothing")])),
        Options::default(),
    );
    let dry = state(&r, "rm").evidence.iter().find(|e| e.check == "dry run").unwrap();
    assert_eq!(dry.result, EvidenceResult::NotApplicable);
    assert!(dry.detail.as_deref().unwrap().contains("expressions"), "{dry:?}");
}

#[test]
fn preconditions_and_assumptions_are_checked_against_the_machine() {
    let mut holds = step("holds", "pwsh", "Get-Date");
    holds["preconditions"] = json!([{"service_state": {"name": "Winmgmt", "state": "running"}}]);
    let mut fails = step("fails", "pwsh", "Get-Date");
    fails["preconditions"] = json!([{"service_state": {"name": "Winmgmt", "state": "stopped"}}]);
    let mut unknown = step("unknown", "pwsh", "Get-Date");
    unknown["preconditions"] = json!([{"fact": {"name": "docker.backend", "equals": "wsl2"}}]);
    let r = validate(&plan(json!([holds, fails, unknown])), Options::default());
    assert_eq!(state(&r, "holds").readiness, Readiness::Ready);
    assert_eq!(state(&r, "fails").readiness, Readiness::Blocked);
    assert_eq!(
        state(&r, "unknown").readiness,
        Readiness::NeedsReview,
        "a fact nobody collected is not guessed"
    );

    let r = validate(
        &plan_with(
            json!([step("a", "pwsh", "Get-Date")]),
            json!([]),
            json!([{"description": "Windows has no C:/keyjutsu-nope", "check": {"path_exists": {"path": "C:/Windows"}}},
                   {"description": "PowerShell 7 is ancient", "check": {"tool_version": {"tool": "cmd.exe", "satisfies": "<5"}}}]),
        ),
        Options::default(),
    );
    assert_eq!(r.assumptions[0].holds, Some(true));
    assert_eq!(r.assumptions[1].holds, Some(false));
    assert!(r.assumptions.len() == 2);
    assert_eq!(state(&r, "a").readiness, Readiness::Blocked, "a false assumption blocks every step");
}

#[test]
fn cmd_steps_are_ready_but_only_lightly_proven() {
    let r = validate(&plan(json!([step("ver", "cmd", "ver")])), Options::default());
    let s = state(&r, "ver");
    assert_eq!(s.readiness, Readiness::Ready);
    assert_eq!(s.proof_level, ProofLevel::Low);
    assert!(s.remaining_uncertainty.iter().any(|u| u.contains("cmd.exe syntax")));
}

#[test]
fn windows_powershell_5_1_is_analysed_in_its_own_shell_without_mangling_text() {
    let r = validate(
        &plan(json!([step("five", "windows_powershell", "Write-Output 'café ✓' | Out-String")])),
        Options::default(),
    );
    let s = state(&r, "five");
    assert_eq!(s.readiness, Readiness::Ready, "{s:#?} {:?}", r.problems);
    assert!(
        s.evidence
            .iter()
            .any(|e| e.check == "shell" && e.detail.as_deref().is_some_and(|d| d.contains("5.1")))
    );
}

#[test]
fn a_missing_working_directory_or_administrator_right_blocks_the_step() {
    let mut s = step("wd", "pwsh", "Get-Date");
    s["working_directory"] = "C:/keyjutsu/no/such/dir".into();
    let mut admin = step("admin", "pwsh", "Get-Date");
    admin["privilege"] = "administrator".into();
    let r = validate(&plan(json!([s, admin])), Options::default());
    assert_eq!(state(&r, "wd").readiness, Readiness::Blocked);
    let admin = state(&r, "admin");
    let elevated =
        admin.evidence.iter().any(|e| e.check == "privilege" && e.result == EvidenceResult::Passed);
    if !elevated {
        assert_eq!(admin.readiness, Readiness::Blocked);
        assert!(failed(admin, "privilege")[0].contains("keyjutsu-broker.exe is not installed"));
    }
}

#[test]
fn recovery_commands_are_checked_too() {
    let mut s = step("with-recovery", "pwsh", "Get-Date");
    s["recovery"] = json!({"strategy": "commands", "commands": [{"text": "Start-Service -Nmae x"}]});
    let r = validate(&plan(json!([s])), Options::default());
    assert_eq!(
        state(&r, "with-recovery").readiness,
        Readiness::Invalid,
        "a recovery that cannot run is no recovery"
    );
}

#[test]
fn a_long_command_line_is_validated_and_recorded() {
    // One line of several thousand characters, as an agent writes a whole
    // small program on one line. Its findings quote its start, and recording
    // used to fail on it when they quoted all of it.
    let line = "$null = Get-Date; ".repeat(300);
    assert!(line.len() > 5000);
    let p = plan(json!([step("long", "pwsh", line.trim())]));
    let r = validate(&p, Options::default());
    let recorded = r.record_in(p.plan(), "2026-09-25T02:00:00Z");
    let text = serde_json::to_string(&recorded).unwrap();
    let back = parse_plan(&text).expect("a plan with a long command line records its validation");
    let st = &back.plan().keyjutsu.as_ref().unwrap().steps["long"];
    assert!(st.evidence.iter().filter_map(|e| e.detail.as_deref()).all(|d| d.chars().count() <= 1000));
    assert!(
        st.evidence.iter().any(|e| e.detail.as_deref().is_some_and(|d| d.contains("…`"))),
        "the cut is marked"
    );
}

#[test]
fn the_report_can_be_recorded_in_a_stored_plan() {
    let p = plan(json!([step("a", "pwsh", "Get-Date")]));
    let r = validate(&p, Options::default());
    let recorded = r.record_in(p.plan(), "2026-09-25T02:00:00Z");
    let text = serde_json::to_string(&recorded).unwrap();
    let back = parse_plan(&text).expect("a recorded report is a valid stored plan");
    assert_eq!(back.plan().keyjutsu.as_ref().unwrap().steps["a"].readiness, Readiness::Ready);
    assert!(keyjutsu_plan::parse_proposal(&text).is_err(), "but never a valid proposal");
}

/// `Start-Process -Wait` waits for everything the program starts, not only
/// the program. A plan printing to PDF with headless Edge hung like this: the
/// PDF was written, and Edge's helpers kept the step waiting. Waiting on the
/// process itself does not.
#[test]
fn start_process_wait_needs_review_but_waiting_on_the_process_does_not() {
    let r = validate(
        &plan(json!([
            step("waits", "pwsh", "Start-Process -FilePath notepad.exe -Wait"),
            step("passthru", "pwsh", "$p = Start-Process -FilePath notepad.exe -PassThru; $p.WaitForExit()"),
            step(
                "wait-process",
                "pwsh",
                "Wait-Process -Name notepad -Timeout 1 -ErrorAction SilentlyContinue"
            )
        ])),
        Options { dry_run: false, ..Options::default() },
    );
    let s = state(&r, "waits");
    assert_eq!(s.readiness, Readiness::NeedsReview, "{s:#?}");
    let said = failed(s, "waiting");
    assert!(
        said.iter().any(|d| d.contains("everything the program starts") && d.contains("WaitForExit")),
        "{said:?}"
    );
    for id in ["passthru", "wait-process"] {
        assert!(failed(state(&r, id), "waiting").is_empty(), "{id}: {:#?}", state(&r, id));
    }
}

/// A plan's steps run in one shell, so `exit` on a step's line ends the
/// shell the rest of the plan needs. Inside a script block it ends only that
/// block, which is fine.
#[test]
fn exit_on_a_steps_line_is_invalid() {
    let r = validate(
        &plan(json!([
            step("exits", "pwsh", "Get-Date; exit 3"),
            step("in-a-block", "pwsh", "& { Get-Date; exit 0 }"),
            step(
                "throws",
                "pwsh",
                "if (-not (Test-Path -LiteralPath C:/Windows)) { throw 'no Windows folder' }"
            )
        ])),
        Options { dry_run: false, ..Options::default() },
    );
    let s = state(&r, "exits");
    assert_eq!(s.readiness, Readiness::Invalid, "{s:#?}");
    let said = failed(s, "exit");
    assert!(said.iter().any(|d| d.contains("ends the shell") && d.contains("throw")), "{said:?}");
    for id in ["in-a-block", "throws"] {
        assert!(failed(state(&r, id), "exit").is_empty(), "{id}: {:#?}", state(&r, id));
    }
}
