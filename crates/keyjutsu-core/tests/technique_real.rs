//! A successful session becomes a Technique, is reopened on a
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
        &keyjutsu_core::runlock::RunLock::unshared(),
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

/// Imported Technique attacks: a crafted export claims trust
/// it was never given, hides characters in its commands, and uses a loose
/// pattern to smuggle flags in through a value.
#[test]
fn an_imported_technique_cannot_claim_trust_or_hide_what_it_runs() {
    let dir = scratch("attacks");
    let store = Store::open(&dir.join("store")).unwrap();
    let t =
        promote(&successful_session(&store), "Check a Windows service", "", &[param("Winmgmt")], AT).unwrap();
    let shared: serde_json::Value = serde_json::from_str(&technique::export(&t)).unwrap();

    // Claims of trust: known-good environments, a validation date, "not
    // imported", and KeyJutsu's own state saying every step is ready.
    let mut claims = shared.clone();
    claims["technique"]["provenance"]["known_good"] = json!([fingerprint::collect(None)]);
    claims["technique"]["provenance"]["last_validated_at"] = json!(AT);
    claims["technique"]["provenance"]["imported"] = json!(false);
    let imported = technique::import(&claims.to_string()).unwrap();
    assert!(imported.provenance.imported);
    assert!(imported.provenance.known_good.is_empty() && imported.provenance.last_validated_at.is_none());
    let draft = instantiate(&imported, &BTreeMap::new()).unwrap();
    assert!(draft.plan().keyjutsu.is_none(), "no validation or approval arrives with it");
    assert!(fit(&imported, &draft, &fingerprint::collect(None)).no_known_good);

    // A command that reads differently from how it runs.
    let mut hidden = shared.clone();
    hidden["technique"]["template"]["steps"][1]["commands"][0]["text"] =
        json!("Write-Output 'checked' # \u{202E}; Remove-Item C:/x");
    let err = technique::import(&hidden.to_string()).unwrap_err();
    assert!(err.contains("U+202E"), "{err}");
    let mut hidden_default = shared.clone();
    hidden_default["technique"]["parameters"][0]["default"] = json!("Win\u{200B}mgmt");
    assert!(technique::import(&hidden_default.to_string()).unwrap_err().contains("U+200B"));
    assert!(instantiate(&t, &values(&[("service_name", "Winmgmt\u{2066}")])).unwrap_err().contains("U+2066"));

    // A loose pattern lets a value add flags. Nothing stops that at the
    // value; what stops it is that the plan is judged by the command it
    // makes, so recursive deletion needs its own typed confirmation.
    let mut loose = shared;
    loose["technique"]["template"]["steps"][1]["commands"][0]["text"] =
        json!("Remove-Item -LiteralPath C:/Users/Public/kj-attack-{{kj:service_name}}");
    loose["technique"]["parameters"][0]["pattern"] = json!(".*");
    let imported = technique::import(&loose.to_string()).unwrap();
    let draft = instantiate(&imported, &values(&[("service_name", "none -Recurse -Force")])).unwrap();
    let (validated, _) = validated(&draft);
    let mut book = ApprovalBook::new();
    assert_eq!(book.approve_all_except_critical(&validated, AT), ["say"], "held for its typed confirmation");
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

/// A Technique named after a long task has an id at the plan's 64-character
/// limit; its draft's id, which adds the revision, must still fit. A draft
/// from "Replace Cats.pdf on the Desktop with a PDF of ten random cat images
/// and open it" was refused as not matching the schema, without saying why.
#[test]
fn a_technique_with_a_long_name_still_makes_a_draft() {
    let dir = scratch("long-name");
    let store = Store::open(&dir.join("store")).unwrap();
    let recorded = successful_session(&store);
    let name = "Replace Cats.pdf on the Desktop with a PDF of ten random cat images and open it";
    let t = promote(&recorded, name, "", &[], AT).unwrap();
    assert_eq!(t.id.len(), 64, "{}", t.id);

    let draft = instantiate(&t, &values(&[])).unwrap_or_else(|e| panic!("{e}"));
    let id = &draft.plan().plan_id;
    assert!(id.len() <= 64 && id.ends_with("-r1"), "{id}");
    assert!(!id.contains("--"), "{id}");
}

/// When a draft is refused, the reason says which part of the plan and why.
#[test]
fn a_refused_draft_says_what_is_wrong() {
    let dir = scratch("refused-draft");
    let store = Store::open(&dir.join("store")).unwrap();
    let recorded = successful_session(&store);
    let mut t = promote(&recorded, "Check a Windows service", "", &[], AT).unwrap();
    t.template.title = Some(String::new());
    let e = instantiate(&t, &values(&[])).unwrap_err();
    assert!(e.contains("/title"), "{e}");
}

/// ADR 0021: a recorded run, cut to one step, comes out redacted, with each
/// step described from the plan it ran and what it printed.
#[test]
fn a_recorded_run_is_exported_with_its_steps_from_the_plan() {
    use keyjutsu_core::recording::{Recorder, prepare_export};
    let dir = scratch("recorded");
    let store = Store::open(&dir.join("store")).unwrap();
    let session = successful_session(&store);
    let rec = Recorder::new(80, 24);
    rec.output("PS> ");
    rec.step_started("look");
    rec.output("Get-Service -Name Winmgmt\r\nRunning  Winmgmt\r\nPS> ");
    rec.step_finished("look", true);
    rec.step_started("say");
    rec.output("Write-Output done token=ghp_0123456789abcdefghijABCDEFGHIJ012345\r\ndone\r\nPS> ");
    rec.step_finished("say", true);
    history::save_recording(&store, &session.id, &rec.finish(&session.task)).unwrap();
    assert!(history::list(&store).unwrap().iter().any(|s| s.id == session.id && s.recorded));

    let whole = history::load_recording(&store, &session.id).unwrap().unwrap();
    let all: String = whole.events.iter().map(|e| e.data.as_str()).collect();
    assert!(!all.contains("ghp_"), "kept redacted: {all}");

    let e = prepare_export(&session, &whole, Some("say"), None, &[]).unwrap();
    assert_eq!(e.steps.len(), 1);
    let say = &e.steps[0];
    assert_eq!(say.step, "say");
    let planned = session.snapshot().unwrap().plan().step("say").unwrap().clone();
    assert_eq!(say.title, planned.title);
    assert_eq!(say.commands, planned.commands.iter().map(|c| c.text.clone()).collect::<Vec<_>>());
    assert!(say.printed.contains("done") && !say.printed.contains("Winmgmt"), "{}", say.printed);
    assert!(e.cast.contains("\"start:say\"") && !e.cast.contains("\"start:look\""), "{}", e.cast);
    assert_eq!(prepare_export(&session, &whole, None, None, &[]).unwrap().steps.len(), 2);
    assert!(prepare_export(&session, &whole, Some("nowhere"), None, &[]).is_err());

    // A name in what it printed and in the plan's commands is masked in the
    // recording and in the guide alike ("Winmgmt" stands in for an account).
    let named = prepare_export(&session, &whole, Some("look"), None, &["winmgmt".to_owned()]).unwrap();
    let look = &named.steps[0];
    assert!(look.commands.iter().all(|c| !c.contains("Winmgmt")), "{:?}", look.commands);
    assert_eq!((look.title.as_str(), look.objective.as_str()), ("Look at *******", "******* is running."));
    assert!(!look.printed.contains("Winmgmt"), "{}", look.printed);
    assert!(!named.cast.to_lowercase().contains("winmgmt"), "{}", named.cast);
    assert!(named.redactions.iter().any(|r| r.starts_with("account name ×")), "{:?}", named.redactions);
}
