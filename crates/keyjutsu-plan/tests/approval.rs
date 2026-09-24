//! Milestone 4: approval binds to exact step hashes, a change invalidates the
//! step and everything after it, critical steps need their typed phrase, and
//! a sealed snapshot refuses to load if anything in it was altered.

#![allow(clippy::unwrap_used)] // Helpers outside #[test] functions may unwrap too.

use keyjutsu_plan::approval::{ApprovalError, SealError, SnapshotError, confirmation_phrase};
use keyjutsu_plan::hash::{EnvironmentFingerprint, FingerprintEntry, affected_by_drift};
use keyjutsu_plan::{
    ApprovalBook, ApprovedSnapshot, StepApproval, ValidPlan, diff, parse_plan, seal, step_hashes,
};
use proptest::prelude::*;
use serde_json::{Value, json};

const AT: &str = "2026-09-25T01:00:00Z";

/// §11's shape: s1 -> s2 -> s3 -> s4 -> s5, s2 -> s6, s3 -> s7.
fn chain() -> Value {
    let step = |n: u32| {
        json!({
            "id": format!("s{n}"), "title": format!("Step {n}"), "objective": "Check something.",
            "kind": "validation", "shell": {"kind": "pwsh"}, "commands": [{"text": format!("Write-Output {n}")}]
        })
    };
    json!({
        "schema_version": "1.0", "plan_id": "p", "task_id": "t",
        "target": {"id": "local", "kind": "local_windows"},
        "agent": {"name": "codex", "version": "1"},
        "steps": (1..=7).map(step).collect::<Vec<_>>(),
        "edges": [
            {"from": "s1", "to": "s2"}, {"from": "s2", "to": "s3"}, {"from": "s3", "to": "s4"},
            {"from": "s4", "to": "s5"}, {"from": "s2", "to": "s6"}, {"from": "s3", "to": "s7"}
        ]
    })
}

fn plan(v: &Value) -> ValidPlan {
    parse_plan(&v.to_string()).unwrap()
}

fn approved(p: &ValidPlan) -> ApprovalBook {
    let mut book = ApprovalBook::new();
    assert!(book.approve_all_except_critical(p, AT).is_empty());
    book
}

fn fingerprint() -> EnvironmentFingerprint {
    EnvironmentFingerprint {
        os: "Windows 11 Pro 25H2".into(),
        build: "26200.9457".into(),
        architecture: "x64".into(),
        shells: vec![FingerprintEntry {
            name: "pwsh".into(),
            path: Some("C:/pwsh.exe".into()),
            version: Some("7.6.6".into()),
        }],
        tools: vec![],
    }
}

fn invalidated(book: &ApprovalBook, p: &ValidPlan) -> Vec<String> {
    let status = book.status(p.plan(), p.graph());
    p.graph()
        .topological_order()
        .filter(|id| matches!(status[*id], StepApproval::Invalidated { .. }))
        .map(str::to_owned)
        .collect()
}

#[test]
fn changing_a_step_invalidates_it_and_everything_after_it() {
    let before = plan(&chain());
    let book = approved(&before);

    let mut v = chain();
    v["steps"][2]["commands"][0]["text"] = "Write-Output changed".into();
    let after = plan(&v);
    assert_eq!(invalidated(&book, &after), ["s3", "s4", "s5", "s7"], "§11's example, exactly");

    // The untouched branch keeps its approval.
    let status = book.status(after.plan(), after.graph());
    assert_eq!(status["s6"], StepApproval::Approved);
    assert_eq!(status["s1"], StepApproval::Approved);

    let err = seal(&after, &book, None, AT).unwrap_err();
    assert_eq!(err, SealError::NotApproved(vec!["s3".into(), "s4".into(), "s5".into(), "s7".into()]));
}

#[test]
fn rewording_a_step_keeps_every_approval() {
    let before = plan(&chain());
    let book = approved(&before);
    let mut v = chain();
    v["steps"][2]["title"] = "A clearer title".into();
    v["steps"][2]["reason"] = "Because.".into();
    assert!(invalidated(&book, &plan(&v)).is_empty());
}

#[test]
fn a_new_branch_condition_invalidates_its_target_and_what_follows() {
    let book = approved(&plan(&chain()));
    let mut v = chain();
    v["edges"][4]["when"] = json!({"fact": {"name": "x", "equals": true}}); // s2 -> s6
    assert_eq!(invalidated(&book, &plan(&v)), ["s6"]);
}

#[test]
fn a_plan_wide_requirement_invalidates_every_step() {
    let book = approved(&plan(&chain()));
    let mut v = chain();
    v["requirements"] = json!([{"name": "docker", "executable": "docker.exe", "version": ">=28"}]);
    assert_eq!(invalidated(&book, &plan(&v)).len(), 7);
}

#[test]
fn hashes_do_not_depend_on_key_order_or_whitespace() {
    let a = plan(&chain());
    let pretty = serde_json::to_string_pretty(&chain()).unwrap();
    let b = parse_plan(&pretty).unwrap();
    assert_eq!(step_hashes(a.plan(), a.graph()), step_hashes(b.plan(), b.graph()));
}

#[test]
fn a_sealed_snapshot_round_trips_exactly() {
    let p = plan(&chain());
    let snap = seal(&p, &approved(&p), Some(fingerprint()), AT).unwrap();
    let json = snap.to_json();
    let loaded = ApprovedSnapshot::from_json(&json).unwrap();
    assert_eq!(loaded, snap);
    assert_eq!(loaded.to_json(), json, "the stored form is stable");
    assert_eq!(loaded.snapshot_hash().len(), 64);
    // Sealing the same approvals at the same moment gives the same hash.
    assert_eq!(
        seal(&p, &approved(&p), Some(fingerprint()), AT).unwrap().snapshot_hash(),
        snap.snapshot_hash()
    );
}

fn tamper(edit: impl FnOnce(&mut Value)) -> SnapshotError {
    let p = plan(&chain());
    let snap = seal(&p, &approved(&p), Some(fingerprint()), AT).unwrap();
    let mut v: Value = serde_json::from_str(&snap.to_json()).unwrap();
    edit(&mut v);
    ApprovedSnapshot::from_json(&v.to_string()).unwrap_err()
}

#[test]
fn any_edit_to_a_stored_snapshot_is_refused() {
    type Edit = Box<dyn FnOnce(&mut Value)>;
    let cases: Vec<(&str, Edit)> = vec![
        ("a command", Box::new(|v| v["plan"]["steps"][2]["commands"][0]["text"] = "Remove-Item C:/x".into())),
        ("a title", Box::new(|v| v["plan"]["steps"][0]["title"] = "Something else".into())),
        ("an approval's hash", Box::new(|v| v["approvals"][0]["step_hash"] = "0".repeat(64).into())),
        (
            "a missing approval",
            Box::new(|v| {
                v["approvals"].as_array_mut().unwrap().pop();
            }),
        ),
        ("the recorded step hashes", Box::new(|v| v["step_hashes"]["s1"] = "1".repeat(64).into())),
        ("the snapshot hash", Box::new(|v| v["snapshot_hash"] = "2".repeat(64).into())),
        ("the fingerprint", Box::new(|v| v["fingerprint"]["build"] = "22000.1".into())),
        ("the seal time", Box::new(|v| v["sealed_at"] = "2027-01-01T00:00:00Z".into())),
        (
            "an added edge",
            Box::new(|v| {
                v["plan"]["edges"].as_array_mut().unwrap().push(json!({"from": "s1", "to": "s6"}));
            }),
        ),
        (
            "a KeyJutsu state section",
            Box::new(|v| {
                v["plan"]["keyjutsu"] = json!({"steps": {}});
            }),
        ),
    ];
    for (what, edit) in cases {
        let err = tamper(edit);
        assert!(matches!(err, SnapshotError::Tampered(_)), "editing {what} gave {err:?}");
    }
}

#[test]
fn a_snapshot_of_another_format_is_refused_by_name() {
    assert!(matches!(
        tamper(|v| v["kind"] = "keyjutsu.snapshot/2".into()),
        SnapshotError::UnsupportedFormat { .. }
    ));
    assert!(matches!(ApprovedSnapshot::from_json("{}"), Err(SnapshotError::NotASnapshot(_))));
    assert!(matches!(tamper(|v| v["extra"] = 1.into()), SnapshotError::NotASnapshot(_)));
}

fn critical_plan() -> Value {
    let mut v = chain();
    v["steps"][6] = json!({
        "id": "s7", "title": "Remove local Docker data", "objective": "Start from a clean data directory.",
        "kind": "command", "shell": {"kind": "pwsh"},
        "commands": [{"text": "Remove-Item -Recurse -Force C:/ProgramData/Docker/data"}],
        "proposed_risk": {"level": "critical", "rationale": "Deletes containers, images and volumes."},
        "reversibility": {"level": "none"}
    });
    v
}

#[test]
fn whole_plan_approval_never_covers_a_critical_step() {
    let p = plan(&critical_plan());
    let mut book = ApprovalBook::new();
    let skipped = book.approve_all_except_critical(&p, AT);
    assert_eq!(skipped, ["s7"]);
    assert!(matches!(seal(&p, &book, None, AT), Err(SealError::NotApproved(s)) if s == ["s7"]));

    let phrase = confirmation_phrase(p.plan().step("s7").unwrap());
    assert_eq!(phrase, "REMOVE LOCAL DOCKER DATA");
    assert_eq!(
        book.approve(&p, "s7", AT, None),
        Err(ApprovalError::ConfirmationRequired { step: "s7".into(), phrase: phrase.clone() })
    );
    assert!(matches!(
        book.approve(&p, "s7", AT, Some("yes")),
        Err(ApprovalError::ConfirmationMismatch { .. })
    ));
    assert!(
        matches!(
            book.approve(&p, "s7", AT, Some("remove local docker data")),
            Err(ApprovalError::ConfirmationMismatch { .. })
        ),
        "the phrase is typed exactly"
    );
    book.approve(&p, "s7", AT, Some(" REMOVE LOCAL DOCKER DATA ")).unwrap();

    let snap = seal(&p, &book, None, AT).unwrap();
    ApprovedSnapshot::from_json(&snap.to_json()).unwrap();

    // Stripping the typed confirmation out of the stored snapshot is caught,
    // even by a forger who recomputes the snapshot hash afterwards: the hash
    // is unkeyed, so it cannot be what stops this.
    let mut v: Value = serde_json::from_str(&snap.to_json()).unwrap();
    for a in v["approvals"].as_array_mut().unwrap() {
        a.as_object_mut().unwrap().remove("confirmation");
    }
    let forged_hash = keyjutsu_plan::hash::hash_value(&json!({
        "kind": "keyjutsu.snapshot/1",
        "plan": v["plan"],
        "step_hashes": v["step_hashes"],
        "approvals": v["approvals"],
        "fingerprint": Value::Null,
        "sealed_at": v["sealed_at"],
    }));
    v["snapshot_hash"] = forged_hash.into();
    let err = ApprovedSnapshot::from_json(&v.to_string()).unwrap_err();
    assert!(
        matches!(&err, SnapshotError::Tampered(m) if m.contains("typed confirmation")),
        "the confirmation check itself must refuse it: {err:?}"
    );
}

#[test]
fn drift_puts_only_the_affected_steps_in_question() {
    let p = plan(&chain());
    let before = fingerprint();
    let mut after = fingerprint();
    after.shells[0].version = Some("7.7.0".into());
    let drifts = before.drift(&after);
    assert_eq!(affected_by_drift(p.plan(), p.graph(), &drifts).len(), 7, "every step uses pwsh");

    let mut v = chain();
    v["steps"][5]["shell"] = json!({"kind": "cmd"});
    v["steps"][5]["commands"] = json!([{"text": "ver"}]);
    let p = plan(&v);
    let mut cmd_after = fingerprint();
    cmd_after.shells.push(FingerprintEntry {
        name: "cmd".into(),
        path: Some("C:/cmd.exe".into()),
        version: None,
    });
    let drifts = before.drift(&cmd_after);
    assert_eq!(affected_by_drift(p.plan(), p.graph(), &drifts), ["s6"], "only the cmd step");

    let mut os = fingerprint();
    os.build = "26300.1".into();
    assert_eq!(affected_by_drift(p.plan(), p.graph(), &before.drift(&os)).len(), 7);
}

/// A step-level edit, chosen by index.
fn apply_edit(v: &mut Value, step: usize, edit: u8) {
    let s = &mut v["steps"][step];
    match edit {
        0 => s["commands"][0]["text"] = "Write-Output edited".into(),
        1 => s["title"] = "Edited title".into(),
        2 => s["shell"] = json!({"kind": "windows_powershell"}),
        3 => s["privilege"] = "administrator".into(),
        4 => s["objective"] = "Edited objective.".into(),
        5 => s["internal_validation"] = json!([{"exit_code": {"equals": 0}}]),
        _ => s["working_directory"] = "C:/work".into(),
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 500, ..ProptestConfig::default() })]

    /// The two answers to "what does this change affect?" must never disagree:
    /// the steps whose hashes change are exactly the steps `diff` reports.
    #[test]
    fn hashes_and_diff_agree_on_what_a_change_affects(
        edits in prop::collection::vec((0usize..7, 0u8..7), 1..4)
    ) {
        let old = plan(&chain());
        let mut v = chain();
        for (step, edit) in edits {
            apply_edit(&mut v, step, edit);
        }
        let new = plan(&v);
        let (h_old, h_new) = (step_hashes(old.plan(), old.graph()), step_hashes(new.plan(), new.graph()));
        let changed: Vec<String> = new.graph().topological_order()
            .filter(|id| h_old.get(*id) != h_new.get(*id))
            .map(str::to_owned)
            .collect();
        let d = diff(old.plan(), new.plan(), new.graph());
        prop_assert_eq!(changed, d.affected);
    }
}

fn with_validation(mut v: Value, readiness: &[(&str, &str, &str)]) -> Value {
    let mut steps = serde_json::Map::new();
    for (id, ready, risk) in readiness {
        steps.insert(
            (*id).into(),
            json!({"readiness": ready, "proof_level": "MEDIUM", "assessed_risk": risk}),
        );
    }
    v["keyjutsu"] = json!({"revision": 1, "steps": steps});
    v
}

#[test]
fn a_validated_plan_only_seals_when_every_step_is_ready() {
    let mut rows: Vec<(&str, &str, &str)> =
        ["s1", "s2", "s3", "s4", "s5", "s6", "s7"].iter().map(|s| (*s, "READY", "low")).collect();
    rows[3].1 = "BLOCKED";
    let p = plan(&with_validation(chain(), &rows));
    let book = approved(&p);
    assert_eq!(seal(&p, &book, None, AT), Err(SealError::NotReady(vec!["s4".into()])));
}

#[test]
fn keyjutsus_own_critical_rating_needs_the_typed_phrase_even_if_the_agent_said_low() {
    let rows: Vec<(&str, &str, &str)> = ["s1", "s2", "s3", "s4", "s5", "s6", "s7"]
        .iter()
        .map(|s| (*s, "READY", if *s == "s5" { "critical" } else { "low" }))
        .collect();
    let mut v = with_validation(chain(), &rows);
    v["steps"][4]["proposed_risk"] = json!({"level": "low", "rationale": "Harmless."});
    let p = plan(&v);
    let mut book = ApprovalBook::new();
    assert_eq!(book.approve_all_except_critical(&p, AT), ["s5"]);
    book.approve(&p, "s5", AT, Some("STEP 5")).unwrap();
    let snap = seal(&p, &book, None, AT).unwrap();
    // The assessment is sealed with the plan, so the check survives a reload.
    let loaded = ApprovedSnapshot::from_json(&snap.to_json()).unwrap();
    assert_eq!(
        loaded.plan().keyjutsu.as_ref().unwrap().steps["s5"].assessed_risk,
        Some(keyjutsu_plan::model::RiskLevel::Critical)
    );
}
