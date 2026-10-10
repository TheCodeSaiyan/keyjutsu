//! `keyjutsu profile` and the `--profile` option, through the real binary.

#![allow(clippy::unwrap_used)] // Helpers outside #[test] functions may unwrap too.
#![allow(clippy::disallowed_methods)] // Tests start programs directly; no window matters here.

use std::path::{Path, PathBuf};
use std::process::Command;

/// A machine of the test's own: a profile home, a store, and a folder to
/// run in, apart from the operator's and from every other test's.
struct Machine {
    root: PathBuf,
}

impl Machine {
    fn new(name: &str) -> Self {
        let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join("profile-cli").join(name);
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("work")).unwrap();
        Self { root }
    }

    fn work(&self) -> PathBuf {
        self.root.join("work")
    }

    fn run_in(&self, dir: &Path, args: &[&str]) -> (i32, String, String) {
        let out = Command::new(env!("CARGO_BIN_EXE_keyjutsu"))
            .args(args)
            .current_dir(dir)
            .env("KEYJUTSU_PROFILE_HOME", self.root.join("home"))
            .env("KEYJUTSU_STORE", self.root.join("store"))
            .output()
            .unwrap();
        (
            out.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }

    fn run(&self, args: &[&str]) -> (i32, String, String) {
        self.run_in(&self.work(), args)
    }
}

fn example(name: &str) -> String {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/examples").join(name).display().to_string()
}

#[test]
fn the_six_profiles_are_listed_with_what_each_extends() {
    let (code, out, _) = Machine::new("list").run(&["profile", "list"]);
    assert_eq!(code, 0);
    for line in
        ["developer          Developer\n", "audit-only         Audit only, extends security-response\n"]
    {
        assert!(out.contains(line), "{out}");
    }
    assert_eq!(out.lines().count(), 6, "{out}");
}

#[test]
fn show_names_where_each_rule_comes_from() {
    let (code, out, _) = Machine::new("show").run(&["profile", "show", "security-response"]);
    assert_eq!(code, 0);
    assert!(out.contains("Chain: security-response (built into KeyJutsu) → it-operations"), "{out}");
    assert!(out.contains("refused: Format-Volume"), "{out}");
    assert!(out.contains("from helpdesk"), "{out}");
    assert!(out.contains("This plan is for Security response work"), "{out}");
}

#[test]
fn an_unknown_profile_is_refused_before_anything_else() {
    let (code, _, err) = Machine::new("unknown").run(&[
        "plan",
        "validate",
        &example("check-a-service.json"),
        "--profile",
        "lenient",
    ]);
    assert_eq!(code, 2);
    assert!(err.contains("there is no profile called \"lenient\""), "{err}");
}

#[test]
fn planning_needs_a_profile_and_the_default_is_kept_once_chosen() {
    let m = Machine::new("default");
    // Nothing is sent: the profile is decided before the agent is even found.
    let propose = ["plan", "propose", "Tidy up", "--agent", "codex", "--out", "plan.json"];
    let (code, _, err) = m.run(&propose);
    assert_eq!(code, 2, "{err}");
    assert!(err.contains("no work profile is set for this machine"), "{err}");

    let (code, out, _) = m.run(&["profile", "default", "it-operations"]);
    assert_eq!(code, 0, "{out}");
    let (_, out, _) = m.run(&["profile", "default"]);
    assert_eq!(out.trim(), "it-operations");
    let (_, out, _) = m.run(&["profile", "which"]);
    assert!(out.contains("Work profile: IT Operations (it-operations), this machine's default."), "{out}");
}

#[cfg(windows)]
#[test]
fn validate_applies_the_profile_that_applies_here() {
    let m = Machine::new("validate");
    let plan = example("clear-build-cache.json");
    // Nothing set: checking an existing plan goes ahead without a profile, and says so.
    let (_, out, err) = m.run(&["plan", "validate", &plan, "--no-dry-run"]);
    assert!(!out.contains("work profile"), "{out}");
    assert!(err.contains("No work profile is set, so none is applied"), "{err}");

    m.run(&["profile", "default", "helpdesk"]);
    let (code, out, err) = m.run(&["plan", "validate", &plan, "--no-dry-run"]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("✕ work profile: Helpdesk needs a recovery"), "{out}");
    assert!(err.contains("Work profile: Helpdesk (helpdesk), this machine's default."), "{err}");

    // --profile overrides it, and says what it overrode.
    let (_, out, err) = m.run(&["plan", "validate", &plan, "--no-dry-run", "--profile", "it-operations"]);
    assert!(out.contains("IT Operations needs a recovery"), "{out}");
    assert!(err.contains("chosen by hand, instead of helpdesk"), "{err}");
}

#[test]
fn a_less_strict_folder_waits_for_confirmation() {
    let m = Machine::new("confirm");
    m.run(&["profile", "default", "it-operations"]);
    std::fs::create_dir_all(m.work().join(".keyjutsu")).unwrap();
    std::fs::write(m.work().join(".keyjutsu/profile.toml"), "default = \"developer\"\n").unwrap();

    let (code, _, err) = m.run(&["plan", "propose", "Tidy up", "--agent", "codex", "--out", "plan.json"]);
    assert_eq!(code, 2);
    assert!(err.contains("less strict than this machine's it-operations"), "{err}");

    let (code, out, _) = m.run(&["profile", "confirm"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.starts_with("Confirmed: plans made in"), "{out}");
    let (_, out, _) = m.run(&["profile", "which"]);
    assert!(out.contains("Developer (developer), set by"), "{out}");
    assert!(out.contains("which you confirmed"), "{out}");
}

#[test]
fn a_broken_folder_stops_planning_until_a_profile_is_picked() {
    let m = Machine::new("broken");
    m.run(&["profile", "default", "developer"]);
    std::fs::create_dir_all(m.work().join(".keyjutsu")).unwrap();
    std::fs::write(m.work().join(".keyjutsu/profile.toml"), "default = \"developer\"\n").unwrap();
    std::fs::write(m.work().join(".keyjutsu/notes.txt"), "stray").unwrap();
    let (code, out, _) = m.run(&["profile", "which"]);
    assert_eq!(code, 1);
    assert!(out.contains("notes.txt is not something KeyJutsu reads"), "{out}");
    let (code, _, err) = m.run(&["plan", "validate", &example("check-a-service.json"), "--no-dry-run"]);
    assert_eq!(code, 2, "{err}");
    assert!(err.contains("--profile NAME chooses one by hand"), "{err}");
}

#[cfg(windows)]
#[test]
fn it_operations_asks_for_a_high_risk_steps_own_confirmation() {
    let m = Machine::new("approve");
    let plan = serde_json::json!({
        "schema_version": "1.0", "plan_id": "spooler", "task_id": "t",
        "target": {"id": "local", "kind": "local_windows"},
        "agent": {"name": "codex", "version": "1"},
        "steps": [{
            "id": "stop-spooler", "title": "Stop the spooler", "objective": "The spooler is stopped.",
            "kind": "command", "shell": {"kind": "pwsh"},
            "commands": [{"text": "Stop-Service -Name Spooler"}],
            "recovery": {"strategy": "commands", "commands": [{"text": "Start-Service -Name Spooler"}]}
        }]
    });
    let file = m.work().join("spooler.json");
    std::fs::write(&file, plan.to_string()).unwrap();
    let file = file.display().to_string();
    let out = m.work().join("spooler.snapshot.json").display().to_string();
    let approve = |extra: &[&str]| {
        let mut args =
            vec!["plan", "approve", &file, "--out", &out, "--no-dry-run", "--profile", "it-operations"];
        args.extend_from_slice(extra);
        m.run(&args)
    };

    let (code, _, err) = approve(&[]);
    assert_eq!(code, 1, "{err}");
    assert!(err.contains("need their own typed confirmation"), "{err}");
    assert!(
        err.contains("CONFIRM          stop-spooler: Stop the spooler (IT Operations asks for its own confirmation from high risk)"),
        "{err}"
    );
    assert!(!Path::new(&out).exists());

    let (code, stdout, err) = approve(&["--confirm", "stop-spooler=STOP THE SPOOLER"]);
    assert_eq!(code, 0, "{stdout}{err}");
    let sealed: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&out).unwrap()).unwrap();
    let recorded = sealed.pointer("/plan/keyjutsu/work_profile").unwrap_or(&serde_json::Value::Null);
    assert_eq!(recorded["name"], "it-operations", "{sealed:#}");
    assert_eq!(recorded["why"], "chosen by hand; no default was set");
}
