//! Milestone 17, IPC attacks: whatever arrives on the broker's pipe, in
//! whatever order, it runs a step only for a client that proved the secret,
//! and only the approved Administrator step at its approved hash. The
//! requests are built from the real ones, forged, mutated and mixed with
//! garbage; a counting runner stands in for the elevated shell.

#![cfg(windows)]
#![allow(clippy::unwrap_used)] // Helpers outside #[test] functions may unwrap too.

use std::sync::{Arc, Mutex, OnceLock};

use keyjutsu_broker::{Broker, PROTOCOL, Request, Response, read_frame};
use keyjutsu_core::elevation::ElevatedRun;
use keyjutsu_core::plan::{ApprovalBook, ApprovedSnapshot, ValidPlan, parse_plan, seal};
use keyjutsu_core::validation::{Options, validate};
use proptest::prelude::*;
use serde_json::json;

const AT: &str = "2026-09-25T12:00:00Z";
const SECRET: &str = "0123456789abcdef0123456789abcdef";

fn snapshot() -> &'static ApprovedSnapshot {
    static SNAP: OnceLock<ApprovedSnapshot> = OnceLock::new();
    SNAP.get_or_init(|| {
        let draft = parse_plan(
            &json!({
                "schema_version": "1.0", "plan_id": "p", "task_id": "t",
                "target": {"id": "local", "kind": "local_windows"},
                "agent": {"name": "codex", "version": "1"},
                "steps": [
                    {"id": "admin", "title": "Admin", "objective": "Needs Administrator.", "kind": "command",
                     "shell": {"kind": "pwsh"}, "privilege": "administrator",
                     "commands": [{"text": "Get-Service -Name Winmgmt"}]},
                    {"id": "plain", "title": "Plain", "objective": "Does not.", "kind": "command",
                     "shell": {"kind": "pwsh"}, "commands": [{"text": "Get-Date"}]}
                ]
            })
            .to_string(),
        )
        .unwrap();
        let report = validate(&draft, Options { dry_run: false, broker_available: true });
        let v = ValidPlan::revalidate(report.record_in(draft.plan(), AT), false).unwrap();
        let mut book = ApprovalBook::new();
        assert!(book.approve_all_except_critical(&v, AT).is_empty());
        seal(&v, &book, None, AT).unwrap()
    })
}

fn to_bytes(r: &Request) -> Vec<u8> {
    serde_json::to_vec(r).unwrap()
}

/// One message on the pipe.
fn message() -> impl Strategy<Value = Vec<u8>> {
    let snap = snapshot();
    let hash = snap.snapshot_hash().to_owned();
    let admin = snap.step_hashes()["admin"].clone();
    let plain = snap.step_hashes()["plain"].clone();
    let good_hello = to_bytes(&Request::Hello { protocol: PROTOCOL, secret: SECRET.into() });
    let good_run = to_bytes(&Request::RunStep {
        snapshot_hash: hash.clone(),
        step: "admin".into(),
        step_hash: admin.clone(),
    });
    let real: Vec<Vec<u8>> = vec![
        good_hello.clone(),
        to_bytes(&Request::Hello { protocol: PROTOCOL, secret: "0".repeat(32) }),
        to_bytes(&Request::Hello { protocol: PROTOCOL, secret: String::new() }),
        to_bytes(&Request::Hello { protocol: PROTOCOL + 1, secret: SECRET.into() }),
        good_run.clone(),
        to_bytes(&Request::RunStep {
            snapshot_hash: hash.clone(),
            step: "admin".into(),
            step_hash: plain.clone(),
        }),
        to_bytes(&Request::RunStep { snapshot_hash: hash.clone(), step: "plain".into(), step_hash: plain }),
        to_bytes(&Request::RunStep {
            snapshot_hash: "0".repeat(64),
            step: "admin".into(),
            step_hash: admin.clone(),
        }),
        to_bytes(&Request::RunStep {
            snapshot_hash: hash.clone(),
            step: "nope".into(),
            step_hash: admin.clone(),
        }),
        to_bytes(&Request::Goodbye),
        // Smuggling a command alongside a real request.
        serde_json::to_vec(&json!({"kind": "run_step", "snapshot_hash": hash, "step": "admin",
                                   "step_hash": admin, "command": "Stop-Computer"}))
        .unwrap(),
        serde_json::to_vec(&json!({"kind": "run_command", "command": "Stop-Computer"})).unwrap(),
        serde_json::to_vec(&json!({"kind": "hello", "protocol": PROTOCOL, "secret": SECRET, "admin": true}))
            .unwrap(),
        b"null".to_vec(),
        b"[]".to_vec(),
        Vec::new(),
    ];
    let mutated =
        (prop::sample::select(vec![good_hello, good_run]), any::<prop::sample::Index>(), any::<u8>())
            .prop_map(|(mut m, at, b)| {
                let i = at.index(m.len());
                m[i] = b;
                m
            });
    prop_oneof![
        4 => prop::sample::select(real),
        2 => mutated,
        1 => prop::collection::vec(any::<u8>(), 0..64),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 3000, ..ProptestConfig::default() })]

    #[test]
    fn nothing_on_the_pipe_runs_anything_but_the_approved_step_for_an_authenticated_client(
        messages in prop::collection::vec(message(), 1..12)
    ) {
        let snap = snapshot();
        let ran: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let seen = ran.clone();
        let mut broker = Broker::new(
            snap.clone(),
            SECRET.into(),
            Box::new(move |_, step| {
                seen.lock().unwrap().push(step.id.clone());
                Ok(ElevatedRun { outcomes: Vec::new(), output: String::new() })
            }),
        );
        let mut authenticated = false;
        for m in &messages {
            let before = ran.lock().unwrap().len();
            let response = broker.handle(m);
            let parsed: Option<Request> = serde_json::from_slice(m).ok();
            if let Some(Request::Hello { protocol, secret }) = &parsed
                && *protocol == PROTOCOL && secret == SECRET
            {
                authenticated = true;
                prop_assert!(matches!(response, Response::Welcome { .. }), "{:?}", response);
            }
            let did_run = ran.lock().unwrap().len() > before;
            if did_run {
                prop_assert!(authenticated, "ran for a client that never proved the secret");
                let Some(Request::RunStep { snapshot_hash, step, step_hash }) = parsed else {
                    return Err(TestCaseError::fail("ran for something that was not a RunStep"));
                };
                prop_assert_eq!(snapshot_hash.as_str(), snap.snapshot_hash());
                prop_assert_eq!(step.as_str(), "admin");
                prop_assert_eq!(&step_hash, &snap.step_hashes()["admin"]);
                prop_assert!(matches!(response, Response::StepDone { .. }), "{:?}", response);
            } else {
                prop_assert!(!matches!(response, Response::StepDone { .. }), "{:?}", response);
            }
        }
        prop_assert!(ran.lock().unwrap().iter().all(|s| s == "admin"));
    }

    /// A frame's length prefix is untrusted: however large it claims to be,
    /// no more than the limit is allocated, and the stream never panics.
    #[test]
    fn any_byte_stream_is_read_as_frames_or_refused(bytes in prop::collection::vec(any::<u8>(), 0..64)) {
        let mut r = std::io::Cursor::new(bytes);
        for _ in 0..8 {
            match read_frame(&mut r) {
                Ok(Some(frame)) => prop_assert!(frame.len() <= 1024 * 1024),
                Ok(None) | Err(_) => break,
            }
        }
    }
}

#[test]
fn a_frame_claiming_four_gigabytes_is_refused_before_anything_is_allocated() {
    let mut r = std::io::Cursor::new(u32::MAX.to_le_bytes().to_vec());
    assert!(read_frame(&mut r).unwrap_err().to_string().contains("too large"));
}

/// The property above is only worth something if the genuine sequence does
/// run the step; otherwise "nothing ran" would pass for a broker that never
/// runs anything.
#[test]
fn the_genuine_sequence_runs_the_approved_step() {
    let snap = snapshot();
    let ran = Arc::new(Mutex::new(0));
    let count = ran.clone();
    let mut broker = Broker::new(
        snap.clone(),
        SECRET.into(),
        Box::new(move |_, _| {
            *count.lock().unwrap() += 1;
            Ok(ElevatedRun { outcomes: Vec::new(), output: String::new() })
        }),
    );
    let hello = broker.handle(&to_bytes(&Request::Hello { protocol: PROTOCOL, secret: SECRET.into() }));
    assert!(matches!(hello, Response::Welcome { .. }), "{hello:?}");
    let run = broker.handle(&to_bytes(&Request::RunStep {
        snapshot_hash: snap.snapshot_hash().into(),
        step: "admin".into(),
        step_hash: snap.step_hashes()["admin"].clone(),
    }));
    assert!(matches!(run, Response::StepDone { .. }), "{run:?}");
    assert_eq!(*ran.lock().unwrap(), 1);
}
