//! Work profiles against real shells: commands are matched as PowerShell
//! parses them, nested code is unpacked, and each rule fails a step the way
//! it says. Nothing here runs a command a plan names: every test validates
//! with dry runs off.

#![cfg(windows)]
#![allow(clippy::unwrap_used)] // Helpers outside #[test] functions may unwrap too.

use keyjutsu_plan::model::{EvidenceResult, Readiness, RiskLevel, StepState};
use keyjutsu_plan::{ValidPlan, parse_plan};
use keyjutsu_validation::profile::{Catalogue, Origin};
use keyjutsu_validation::{Options, Report, validate};
use serde_json::{Value, json};

fn plan(steps: Value) -> ValidPlan {
    let v = json!({
        "schema_version": "1.0", "plan_id": "p", "task_id": "t",
        "target": {"id": "local", "kind": "local_windows"},
        "agent": {"name": "codex", "version": "1"},
        "steps": steps
    });
    parse_plan(&v.to_string()).unwrap_or_else(|e| panic!("{e:?}"))
}

fn step(id: &str, shell: &str, command: &str) -> Value {
    json!({"id": id, "title": id, "objective": "Test.", "kind": "command", "shell": {"kind": shell}, "commands": [{"text": command}]})
}

fn under(profile: &str, catalogue: &Catalogue, steps: Value) -> Report {
    let profile = catalogue.resolve(profile).unwrap();
    validate(&plan(steps), Options { dry_run: false, profile: Some(profile), ..Options::default() })
}

fn built_in(profile: &str, steps: Value) -> Report {
    under(profile, &Catalogue::built_in(), steps)
}

fn state<'a>(r: &'a Report, id: &str) -> &'a StepState {
    r.steps.get(id).unwrap_or_else(|| panic!("no state for {id}"))
}

fn breaches(s: &StepState) -> Vec<String> {
    s.evidence
        .iter()
        .filter(|e| e.check == "work profile" && e.result == EvidenceResult::Failed)
        .map(|e| e.detail.clone().unwrap_or_default())
        .collect()
}

/// What `pwsh -EncodedCommand` expects: Base64 of the UTF-16LE text.
fn encode(s: &str) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes: Vec<u8> = s.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n = chunk.iter().enumerate().fold(0u32, |n, (i, b)| n | (*b as u32) << (16 - 8 * i));
        for i in 0..4 {
            out.push(if i <= chunk.len() {
                ALPHABET[(n >> (18 - 6 * i) & 63) as usize] as char
            } else {
                '='
            });
        }
    }
    out
}

#[test]
fn a_denied_command_is_blocked_however_it_is_wrapped() {
    let wrapped = [
        ("direct", "pwsh", "Format-Volume -DriveLetter Z -FileSystem NTFS".to_owned()),
        ("qualified", "pwsh", "Storage\\Format-Volume -DriveLetter Z".to_owned()),
        (
            "encoded",
            "pwsh",
            format!("pwsh -NoProfile -EncodedCommand {}", encode("Format-Volume -DriveLetter Z")),
        ),
        ("command", "pwsh", "powershell.exe -NoProfile -Command \"Clear-Disk -Number 9\"".to_owned()),
        ("iex", "pwsh", "Invoke-Expression 'Format-Volume -DriveLetter Z'".to_owned()),
        ("cmd-c", "pwsh", "cmd.exe /d /c \"format Z: /q\"".to_owned()),
        ("cmd-shell", "cmd", "echo going & format Z: /q".to_owned()),
        ("two-deep", "cmd", "cmd /c pwsh -NoProfile -c \"Format-Volume -DriveLetter Z\"".to_owned()),
        ("started", "pwsh", "Start-Process -FilePath 'diskpart.exe' -ArgumentList '/s','x.txt'".to_owned()),
    ];
    let r = built_in(
        "helpdesk",
        Value::Array(wrapped.iter().map(|(id, shell, text)| step(id, shell, text)).collect()),
    );
    for (id, _, text) in &wrapped {
        let s = state(&r, id);
        // Blocked by the deny rule; a critical step also lacks the recovery
        // Helpdesk asks for, and INVALID ranks above BLOCKED.
        assert!(matches!(s.readiness, Readiness::Blocked | Readiness::Invalid), "{id}: {text}\n{s:#?}");
        assert!(
            breaches(s).iter().any(|b| b.contains("is refused under Helpdesk")),
            "{id}: {:?}",
            breaches(s)
        );
    }
}

#[test]
fn an_alias_is_matched_by_what_it_stands_for() {
    let mut c = Catalogue::built_in();
    c.add(
        Origin::Machine("test.toml".into()),
        "name = \"no-listing\"\ntitle = \"No listing\"\nextends = \"developer\"\n\
         [[validation.deny]]\ncommands = [\"Get-ChildItem\"]\nreason = \"lists folders\"",
    )
    .unwrap();
    let r = under(
        "no-listing",
        &c,
        json!([step("alias", "pwsh", "gci C:/Windows"), step("other", "pwsh", "Get-Date")]),
    );
    assert_eq!(state(&r, "alias").readiness, Readiness::Blocked, "{:#?}", state(&r, "alias"));
    assert!(breaches(state(&r, "alias"))[0].contains("`get-childitem`"));
    assert_eq!(state(&r, "other").readiness, Readiness::Ready, "{:#?}", state(&r, "other"));
}

#[test]
fn opaque_code_is_refused_only_where_the_profile_says() {
    let steps = json!([
        step("computed", "pwsh", "$tool = 'whoami'; & $tool"),
        step("piped", "pwsh", "'Get-Date' | Invoke-Expression"),
        step("script", "pwsh", "pwsh -NoProfile -File C:/Windows/not-there.ps1"),
        step("plain", "pwsh", "Get-Date")
    ]);
    let strict = built_in("helpdesk", steps.clone());
    for id in ["computed", "piped", "script"] {
        let s = state(&strict, id);
        assert!(matches!(s.readiness, Readiness::Blocked | Readiness::Invalid), "{id}: {s:#?}");
        assert!(breaches(s).iter().any(|b| b.contains("cannot see into")), "{id}: {:?}", breaches(s));
    }
    assert!(breaches(state(&strict, "plain")).is_empty());

    let relaxed = built_in("developer", steps);
    for id in ["computed", "piped", "script", "plain"] {
        let b = breaches(state(&relaxed, id));
        assert!(!b.iter().any(|b| b.contains("cannot see into")), "{id}: {b:?}");
    }
}

#[test]
fn a_change_needs_a_recovery_where_the_profile_requires_one() {
    let folder = std::env::temp_dir().join("keyjutsu-profile-test-never-created");
    let make = format!("New-Item -ItemType Directory -Path '{}'", folder.display());
    let mut recovered = step("recovered", "pwsh", &make);
    recovered["recovery"] = json!({"strategy": "commands", "commands": [{"text": format!("Remove-Item -LiteralPath '{}'", folder.display())}]});
    let mut none = step("none", "pwsh", &make);
    none["recovery"] = json!({"strategy": "none"});
    let steps = json!([step("bare", "pwsh", &make), none, recovered]);

    let ops = built_in("it-operations", steps.clone());
    for id in ["bare", "none"] {
        let s = state(&ops, id);
        assert_eq!(s.readiness, Readiness::Invalid, "{id}: {s:#?}");
        assert!(
            breaches(s)[0].contains("needs a recovery for every step at normal risk"),
            "{:?}",
            breaches(s)
        );
    }
    assert!(breaches(state(&ops, "recovered")).is_empty(), "{:?}", breaches(state(&ops, "recovered")));

    // Developer asks for one only from critical risk.
    let dev = built_in("developer", steps);
    assert!(breaches(state(&dev, "bare")).is_empty());
}

#[test]
fn audit_only_allows_nothing_that_changes_the_machine() {
    let r = built_in(
        "audit-only",
        json!([
            step("look", "pwsh", "Get-Service -Name Winmgmt"),
            step(
                "touch",
                "pwsh",
                "Set-Content -LiteralPath C:/Windows/Temp/keyjutsu-never-written.txt -Value x"
            )
        ]),
    );
    assert_eq!(state(&r, "look").readiness, Readiness::Ready, "{:#?}", state(&r, "look"));
    assert!(
        state(&r, "look")
            .evidence
            .iter()
            .any(|e| e.check == "work profile" && e.result == EvidenceResult::Passed)
    );
    let touch = state(&r, "touch");
    assert_eq!(touch.readiness, Readiness::Invalid, "{touch:#?}");
    assert!(
        breaches(touch).iter().any(|b| b.contains("Audit only allows nothing above low risk")),
        "{:?}",
        breaches(touch)
    );
}

#[test]
fn a_raise_rule_lifts_the_risk_and_says_why() {
    // Not installed on CI, which does not matter: the name is what is matched.
    let r = built_in(
        "it-operations",
        json!([step("ad", "pwsh", "Set-ADUser -Identity someone -Enabled $false")]),
    );
    let s = state(&r, "ad");
    assert_eq!(s.assessed_risk, Some(RiskLevel::High), "{s:#?}");
    assert!(
        s.risk_reasons.iter().any(|r| r.starts_with("IT Operations: `set-aduser`")),
        "{:?}",
        s.risk_reasons
    );
}

#[test]
fn the_plan_records_the_profile_it_was_validated_under() {
    let profile = Catalogue::built_in().resolve("it-operations").unwrap();
    let p = plan(json!([step("look", "pwsh", "Get-Date")]));
    let options = Options {
        dry_run: false,
        profile: Some(profile),
        profile_context: keyjutsu_validation::ProfileContext {
            why: Some("This machine's default.".into()),
            started_in: Some("D:/work".into()),
            ..Default::default()
        },
        ..Options::default()
    };
    let report = validate(&p, options);
    let recorded = report.record_in(p.plan(), "2026-10-10T12:00:00Z");
    let wp = recorded.keyjutsu.unwrap().work_profile.unwrap();
    assert_eq!((wp.name.as_str(), wp.title.as_str()), ("it-operations", "IT Operations"));
    let chain: Vec<&str> = wp.chain.iter().map(|l| l.name.as_str()).collect();
    assert_eq!(chain, ["it-operations", "helpdesk", "developer"]);
    assert_eq!(wp.why.as_deref(), Some("This machine's default."));
    assert_eq!(wp.confirm_each_from, Some(RiskLevel::High));
    assert_eq!(wp.started_in.as_deref(), Some("D:/work"));
    // Each profile's text is hashed as it was read.
    assert!(wp.chain.iter().all(|l| l.hash.as_ref().is_some_and(|h| h.len() == 64)));

    // Validated again without one, the plan no longer claims one.
    let again = validate(&p, Options { dry_run: false, ..Options::default() }).record_in(p.plan(), "x");
    assert_eq!(again.keyjutsu.unwrap().work_profile, None);
}
