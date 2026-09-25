//! Milestone 14: a successful session becomes a Technique, is reopened on a
//! changed environment, and is held for revalidation or adaptation rather
//! than run because it worked before. Real pwsh, real DPAPI, a store of its
//! own in a scratch folder.

#![cfg(windows)]
#![allow(clippy::unwrap_used)] // Helpers outside #[test] functions may unwrap too.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use keyjutsu_core::execute::{Driver, ExecuteOptions, ForwardingSink, Outcome, execute};
use keyjutsu_core::execution::ExecutionMode;
use keyjutsu_core::fingerprint;
use keyjutsu_core::headless::Collector;
use keyjutsu_core::history::{self, SessionRecord};
use keyjutsu_core::plan::model::Readiness;
use keyjutsu_core::plan::{ApprovalBook, ApprovedSnapshot, ValidPlan, parse_plan, seal};
use keyjutsu_core::store::Store;
use keyjutsu_core::technique::{self, Promote, fit, instantiate, promote};
use keyjutsu_core::terminal::{ProfileMode, ShellKind};
use keyjutsu_core::validation::{Options, validate};
use keyjutsu_core::{Session, SessionOptions};
use serde_json::json;

const AT: &str = "2026-09-25T09:00:00Z";

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("technique").join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn validated(plan: &ValidPlan) -> (ValidPlan, Vec<Readiness>) {
    let report = validate(plan, Options { dry_run: false, ..Options::default() });
    let r = plan.graph().topological_order().map(|id| report.steps[id].readiness).collect();
    (ValidPlan::revalidate(report.record_in(plan.plan(), AT), false).unwrap(), r)
}

/// A session that checks a service, run for real and recorded.
fn successful_session(store: &Store) -> SessionRecord {
    let draft = parse_plan(
        &json!({
            "schema_version": "1.0", "plan_id": "check-winmgmt", "task_id": "t", "title": "Check the Winmgmt service",
            "target": {"id": "local", "kind": "local_windows"},
            "agent": {"name": "claude_code", "version": "2.1.282"},
            "steps": [
                {"id": "look", "title": "Look at Winmgmt", "objective": "Winmgmt is running.", "kind": "validation",
                 "shell": {"kind": "pwsh"},
                 "commands": [{"text": "Get-Service -Name Winmgmt | Select-Object -ExpandProperty Status"}],
                 "internal_validation": [{"service_state": {"name": "Winmgmt", "state": "running"}}]},
                {"id": "say", "title": "Say so", "objective": "A template-looking string is not a parameter.",
                 "kind": "command", "shell": {"kind": "pwsh"}, "depends_on": ["look"],
                 "commands": [{"text": "Write-Output 'Winmgmt checked {{.NotAParameter}}'"}]}
            ]
        })
        .to_string(),
    )
    .unwrap();
    let (v, readiness) = validated(&draft);
    assert!(readiness.iter().all(|r| *r == Readiness::Ready), "{readiness:?}");
    let mut book = ApprovalBook::new();
    assert!(book.approve_all_except_critical(&v, AT).is_empty());
    let snap: ApprovedSnapshot = seal(&v, &book, Some(fingerprint::collect(Some(v.plan()))), AT).unwrap();

    let (tx, events) = channel();
    let sink = Arc::new(ForwardingSink { inner: Arc::new(Collector::new()), events: Mutex::new(tx) });
    let mut o = SessionOptions::new(ShellKind::Pwsh);
    o.profile = ProfileMode::Clean;
    o.intercept_cursor_queries = true;
    let session = Session::open(o, sink).unwrap();
    assert!(session.wait_ready(Duration::from_secs(30)));
    let options = ExecuteOptions { mode: Some(ExecutionMode::Direct), ..ExecuteOptions::default() };
    let (outcome, checkpoint) = execute(
        &Driver { session: &session, events: &events },
        &snap,
        None,
        &options,
        &|| AT.to_owned(),
        &|_| {},
    );
    session.close();
    assert_eq!(outcome, Outcome::Complete);

    let record = SessionRecord {
        id: keyjutsu_core::store::new_id(AT),
        started_at: AT.into(),
        finished_at: AT.into(),
        task: "Check that the Winmgmt service is running".into(),
        agent: snap.plan().agent.clone(),
        snapshot: snap.to_json(),
        checkpoint: Some(checkpoint),
        outcome,
        git: Vec::new(),
    };
    history::save(store, &record).unwrap();
    record
}

fn values(v: &[(&str, &str)]) -> BTreeMap<String, String> {
    v.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect()
}

fn param<'a>(value: &'a str) -> Promote<'a> {
    Promote { name: "service_name", description: "The service to check", value, pattern: None }
}

#[test]
fn a_successful_session_becomes_a_technique_that_is_held_for_revalidation_on_a_changed_machine() {
    let dir = scratch("done-when");
    let store = Store::open(&dir.join("store")).unwrap();
    let recorded = successful_session(&store);

    // Reopened from the encrypted history, exactly as it was.
    let reopened = history::load(&Store::open(&dir.join("store")).unwrap(), &recorded.id).unwrap();
    assert_eq!(reopened, recorded);
    assert_eq!(history::list(&store).unwrap()[0].outcome, "complete");

    // Promoted: the service name becomes a parameter; the Docker-style
    // template text is left alone.
    let t = promote(&reopened, "Check a Windows service", "Is it running?", &[param("Winmgmt")], AT).unwrap();
    let text = serde_json::to_string(&t.template).unwrap();
    assert!(text.contains("{{kj:service_name}}") && !text.contains("Winmgmt"), "{text}");
    assert!(text.contains("{{.NotAParameter}}"));
    assert_eq!(t.provenance.origin_session.as_deref(), Some(recorded.id.as_str()));
    assert_eq!(t.provenance.known_good.len(), 1);
    technique::save(&store, &t).unwrap();

    // Used again on the same machine: no drift, validates READY.
    let now = fingerprint::collect(None);
    let draft = instantiate(&t, &values(&[("service_name", "Winmgmt")])).unwrap();
    let f = fit(&t, &draft, &fingerprint::collect(Some(draft.plan())));
    assert!(f.drifts.is_empty() && f.requires_revalidation.is_empty(), "{f:?}");
    assert!(validated(&draft).1.iter().all(|r| *r == Readiness::Ready));

    // The machine changes: PowerShell is not the version it worked on.
    let mut changed = now.clone();
    for s in &mut changed.shells {
        if s.name == "pwsh" {
            s.version = Some("7.0.0".into());
        }
    }
    let f = fit(&t, &draft, &changed);
    assert!(f.drifts.iter().any(|d| d.what == "shell:pwsh"), "{f:?}");
    assert_eq!(f.requires_revalidation, ["look", "say"], "past success says nothing about this machine");
    let recheck = history::recheck(&reopened, &changed).unwrap();
    assert_eq!(recheck.affected, ["look", "say"]);
}

#[test]
fn an_incompatible_technique_is_adapted_as_a_new_revision_and_the_old_one_is_kept() {
    let dir = scratch("adapt");
    let store = Store::open(&dir.join("store")).unwrap();
    let recorded = successful_session(&store);
    let mut t = promote(&recorded, "Check a Windows service", "", &[param("Winmgmt")], AT).unwrap();
    // As if written for a PowerShell this machine does not have.
    t.template.steps[0].shell.as_mut().unwrap().version = Some(">=99".into());
    technique::save(&store, &t).unwrap();

    let draft = instantiate(&t, &values(&[("service_name", "Winmgmt")])).unwrap();
    assert_eq!(validated(&draft).1[0], Readiness::Blocked, "it cannot run here as it is");

    // Adapted (by the operator or an agent) and approved as a new revision.
    let mut adapted = t.template.clone();
    adapted.steps[0].shell.as_mut().unwrap().version = Some(">=7".into());
    let r2 = technique::revise(&store, &t, adapted, Some(fingerprint::collect(None)), AT).unwrap();
    assert_eq!(r2.revision, 2);
    let draft = instantiate(&r2, &values(&[("service_name", "Winmgmt")])).unwrap();
    assert!(validated(&draft).1.iter().all(|r| *r == Readiness::Ready));

    let all = technique::revisions(&store, &t.id).unwrap();
    assert_eq!(all.len(), 2);
    assert_eq!(all[0], t, "revision 1 is kept exactly as it was");
    assert!(technique::save(&store, &t).is_err(), "a revision is never rewritten");
}

#[test]
fn parameter_values_are_data_not_code() {
    let dir = scratch("params");
    let store = Store::open(&dir.join("store")).unwrap();
    let t =
        promote(&successful_session(&store), "Check a Windows service", "", &[param("Winmgmt")], AT).unwrap();
    for bad in ["Winmgmt'; Remove-Item C:/x", "$(Get-Date)", "a|b", "has space", "x`y"] {
        let err = instantiate(&t, &values(&[("service_name", bad)])).unwrap_err();
        assert!(err.contains("service_name"), "{bad}: {err}");
    }
    assert!(instantiate(&t, &values(&[("nope", "x")])).is_err());

    // Even a pattern that accepts anything cannot let code in.
    let mut loose = t.clone();
    loose.parameters[0].pattern = ".*".into();
    for bad in ["x'; Remove-Item C:/x", "$(Get-Date)", "a|b", "x`y"] {
        assert!(instantiate(&loose, &values(&[("service_name", bad)])).is_err(), "{bad}");
    }
    assert!(
        instantiate(&loose, &values(&[("service_name", "has space")])).is_ok(),
        "the author allowed this"
    );
    let with_default = instantiate(&t, &BTreeMap::new()).unwrap();
    assert!(serde_json::to_string(with_default.plan()).unwrap().contains("Winmgmt"));
}

#[test]
fn a_shared_technique_arrives_as_an_untrusted_draft() {
    let dir = scratch("share");
    let store = Store::open(&dir.join("store")).unwrap();
    let recorded = successful_session(&store);
    let t = promote(&recorded, "Check a Windows service", "", &[param("Winmgmt")], AT).unwrap();

    let shared = technique::export(&t);
    assert!(!shared.contains(&recorded.id), "the originating session stays here");
    assert!(!shared.contains(&fingerprint::collect(None).build), "this machine's details stay here");

    let imported = technique::import(&shared).unwrap();
    assert!(imported.provenance.imported);
    let draft = instantiate(&imported, &BTreeMap::new()).unwrap();
    let f = fit(&imported, &draft, &fingerprint::collect(None));
    assert!(f.no_known_good);
    assert_eq!(f.requires_revalidation, ["look", "say"], "nothing is trusted because it worked elsewhere");

    let mut broken: serde_json::Value = serde_json::from_str(&shared).unwrap();
    broken["technique"]["template"]["steps"][0]["commands"][0]["text"] = json!("Get-Date\rRemove-Item x");
    assert!(technique::import(&broken.to_string()).is_err(), "its plan is checked against the schema");
    assert!(technique::import("{}").is_err());
}

#[test]
fn the_store_is_encrypted_and_records_cannot_be_swapped() {
    let dir = scratch("store");
    let root = dir.join("store");
    let store = Store::open(&root).unwrap();
    let recorded = successful_session(&store);
    let file = root.join("session").join(format!("{}.kje", recorded.id));
    let bytes = std::fs::read(&file).unwrap();
    for plain in ["Winmgmt", "Check that", "claude_code"] {
        assert!(!bytes.windows(plain.len()).any(|w| w == plain.as_bytes()), "{plain} is readable on disk");
    }
    assert!(!std::fs::read(root.join("key.dpapi")).unwrap().is_empty());

    // Moved to another record's name: refused, not read as the other.
    std::fs::copy(&file, root.join("session").join("19990101-000000-0000.kje")).unwrap();
    assert!(history::load(&store, "19990101-000000-0000").is_err());
    // Altered: refused.
    let mut altered = bytes.clone();
    let last = altered.len() - 1;
    altered[last] ^= 1;
    std::fs::write(&file, altered).unwrap();
    assert!(history::load(&store, &recorded.id).is_err());
    assert_eq!(store.clear(history::KIND).unwrap(), 2);
    assert!(history::list(&store).unwrap().is_empty());
}
