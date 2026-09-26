//! Recovery: capture what a step will change before it runs, and put
//! it back when the operator asks.
//!
//! Before a step that declares captures runs, its prior state is recorded in
//! the checkpoint: a file's bytes (copied aside and the copy verified by
//! hash), a registry value and its type, a service's status and start type.
//! If any capture cannot be made and verified, the step does not run: a
//! recovery that was never prepared is not a recovery.
//!
//! Nothing is rolled back automatically. After a failure the operator reviews
//! the recovery plan and confirms it; recovery then restores the captures, or
//! runs the step's approved recovery commands, in reverse order, and checks
//! the result against what was captured. It touches only what a step declared.

use std::path::{Path, PathBuf};
use std::time::Duration;

use keyjutsu_execution::{
    ExecutionMode as EngineMode, PerformanceConfig, StagedScript, StagedStep, StepOutcome,
};
use keyjutsu_plan::approval::ApprovedSnapshot;
use keyjutsu_plan::hash::sha256_hex;
use keyjutsu_plan::model::{Capture, CaptureKind, Privilege, RecoveryStrategy, Step};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::execute::{CheckResult, Checkpoint, Driver, Performed, perform, run_check};

/// The state of one thing a step declared it would change, from just before
/// the step ran.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export, export_to = "recovery/")]
pub enum Captured {
    /// `backup` is a file name in the run's recovery folder.
    File {
        path: String,
        existed: bool,
        sha256: Option<String>,
        backup: Option<String>,
    },
    /// `value_json` is the value as PowerShell's `ConvertTo-Json` wrote it.
    RegistryValue {
        target: String,
        existed: bool,
        value_kind: Option<String>,
        value_json: Option<String>,
    },
    ServiceState {
        name: String,
        exists: bool,
        status: Option<String>,
        start_type: Option<String>,
    },
    /// Recorded so the gap is visible; package versions cannot be restored yet.
    PackageVersion {
        name: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "recovery/")]
pub struct StepCapture {
    pub step: String,
    pub step_hash: String,
    pub captured_at: String,
    pub items: Vec<Captured>,
}

/// Where a run keeps its file backups: next to the checkpoint.
pub fn recovery_dir(checkpoint: &Path) -> PathBuf {
    checkpoint.with_extension("recovery")
}

fn powershell() -> Result<PathBuf, String> {
    keyjutsu_terminal::shell::locate(keyjutsu_terminal::ShellKind::Pwsh)
        .or_else(|| keyjutsu_terminal::shell::locate(keyjutsu_terminal::ShellKind::WindowsPowershell))
        .ok_or_else(|| "PowerShell is not installed".to_owned())
}

/// Run `body` in a profile-free PowerShell with `$req` set from `request`,
/// and read the one JSON document it writes. The request travels as base64,
/// so nothing in it is ever parsed as PowerShell.
fn pwsh_json(request: &Value, body: &str) -> Result<Value, String> {
    let b64 = keyjutsu_validation::process::base64_encode(request.to_string().as_bytes());
    let script = format!(
        "$ErrorActionPreference = 'Stop'\n$req = [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String('{b64}')) | ConvertFrom-Json\n{body}"
    );
    let mut c = std::process::Command::new(powershell()?);
    c.args(["-NoLogo", "-NoProfile", "-NonInteractive", "-EncodedCommand"]);
    c.arg(keyjutsu_terminal::shell::encode_powershell_command(&script));
    let done =
        keyjutsu_validation::process::run(c, "", Duration::from_secs(30)).map_err(|e| e.to_string())?;
    if !done.success {
        return Err(done
            .stderr
            .lines()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("PowerShell failed")
            .to_owned());
    }
    serde_json::from_str(done.stdout.trim()).map_err(|e| format!("unreadable answer from PowerShell: {e}"))
}

/// `HKCU:\Software\Thing\Value` names value `Value` under key
/// `HKCU:\Software\Thing`. Only the two hives a plan can reasonably touch.
pub fn split_registry_target(target: &str) -> Result<(&str, &str), String> {
    let upper = target.to_ascii_uppercase();
    if !(upper.starts_with("HKCU:\\") || upper.starts_with("HKLM:\\")) {
        return Err(format!("`{target}`: a registry capture is HKCU:\\Key\\Value or HKLM:\\Key\\Value"));
    }
    match target.rsplit_once('\\') {
        Some((key, name)) if !name.is_empty() && key.len() > 6 => Ok((key, name)),
        _ => Err(format!("`{target}` names no value")),
    }
}

const REGISTRY_READ: &str = r#"
$r = [ordered]@{ existed = $false; kind = $null; value = $null }
$k = Get-Item -LiteralPath $req.key -ErrorAction SilentlyContinue
if ($k -and ($k.GetValueNames() -contains $req.name)) {
  $r.existed = $true
  $r.kind = $k.GetValueKind($req.name).ToString()
  $v = $k.GetValue($req.name, $null, 'DoNotExpandEnvironmentNames')
  $r.value = ConvertTo-Json -InputObject $v -Compress -Depth 4
}
[pscustomobject]$r | ConvertTo-Json -Compress
"#;

const REGISTRY_WRITE: &str = r#"
if ($req.existed) {
  if (-not (Test-Path -LiteralPath $req.key)) { New-Item -Path $req.key -Force | Out-Null }
  $v = ConvertFrom-Json -InputObject $req.value
  switch ($req.kind) {
    'Binary' { $v = [byte[]]@($v) }
    'MultiString' { $v = [string[]]@($v) }
    'DWord' { $v = [int]$v }
    'QWord' { $v = [long]$v }
  }
  New-ItemProperty -LiteralPath $req.key -Name $req.name -Value $v -PropertyType $req.kind -Force | Out-Null
} elseif (Test-Path -LiteralPath $req.key) {
  Remove-ItemProperty -LiteralPath $req.key -Name $req.name -ErrorAction SilentlyContinue
}
'{}'
"#;

const SERVICE_READ: &str = r#"
$s = Get-Service -Name $req.name -ErrorAction SilentlyContinue
if ($s) { [pscustomobject]@{ exists = $true; status = $s.Status.ToString(); start_type = $s.StartType.ToString() } | ConvertTo-Json -Compress }
else { '{"exists":false}' }
"#;

const SERVICE_WRITE: &str = r#"
Set-Service -Name $req.name -StartupType $req.start_type
if ($req.status -eq 'Running') { Start-Service -Name $req.name } elseif ($req.status -eq 'Stopped') { Stop-Service -Name $req.name }
'{}'
"#;

fn read_registry(target: &str) -> Result<(bool, Option<String>, Option<String>), String> {
    let (key, name) = split_registry_target(target)?;
    let v = pwsh_json(&json!({"key": key, "name": name}), REGISTRY_READ)?;
    let existed = v["existed"].as_bool().unwrap_or(false);
    let kind = v["kind"].as_str().map(str::to_owned);
    if let Some(k) = &kind
        && !["String", "ExpandString", "DWord", "QWord", "MultiString", "Binary"].contains(&k.as_str())
    {
        return Err(format!("`{target}` is a {k} value, which KeyJutsu cannot restore"));
    }
    Ok((existed, kind, v["value"].as_str().map(str::to_owned)))
}

fn read_service(name: &str) -> Result<(bool, Option<String>, Option<String>), String> {
    let v = pwsh_json(&json!({"name": name}), SERVICE_READ)?;
    Ok((
        v["exists"].as_bool().unwrap_or(false),
        v["status"].as_str().map(str::to_owned),
        v["start_type"].as_str().map(str::to_owned),
    ))
}

fn capture_one(c: &Capture, step: &str, index: usize, dir: &Path) -> Result<Captured, String> {
    match c.kind {
        CaptureKind::File => {
            // Exactly the file named: never through a link or junction, which
            // for the broker would mean reading as Administrator whatever a
            // link planted since approval points at.
            let read =
                crate::exact_file::read(&c.target).map_err(|e| format!("cannot read `{}`: {e}", c.target))?;
            let Some(bytes) = read else {
                return Ok(Captured::File {
                    path: c.target.clone(),
                    existed: false,
                    sha256: None,
                    backup: None,
                });
            };
            let sha = sha256_hex(&bytes);
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
            let name = format!("{step}-{index}.bak");
            let backup = dir.join(&name);
            std::fs::write(&backup, &bytes).map_err(|e| format!("cannot back up `{}`: {e}", c.target))?;
            // Verify the copy, not just the write.
            let copied = std::fs::read(&backup).map_err(|e| e.to_string())?;
            if sha256_hex(&copied) != sha {
                return Err(format!("the backup of `{}` does not match the original", c.target));
            }
            Ok(Captured::File {
                path: c.target.clone(),
                existed: true,
                sha256: Some(sha),
                backup: Some(name),
            })
        }
        CaptureKind::RegistryValue => {
            let (existed, value_kind, value_json) = read_registry(&c.target)?;
            Ok(Captured::RegistryValue { target: c.target.clone(), existed, value_kind, value_json })
        }
        CaptureKind::ServiceState => {
            let (exists, status, start_type) = read_service(&c.target)?;
            Ok(Captured::ServiceState { name: c.target.clone(), exists, status, start_type })
        }
        CaptureKind::PackageVersion => Ok(Captured::PackageVersion { name: c.target.clone() }),
    }
}

/// Capture everything `step` declares, or say why it cannot be done.
pub fn capture_step(step: &Step, step_hash: &str, dir: &Path, at: String) -> Result<StepCapture, String> {
    let captures = step.recovery.as_ref().map(|r| r.capture.as_slice()).unwrap_or_default();
    let mut items = Vec::new();
    for (i, c) in captures.iter().enumerate() {
        items.push(capture_one(c, &step.id, i, dir)?);
    }
    Ok(StepCapture { step: step.id.clone(), step_hash: step_hash.to_owned(), captured_at: at, items })
}

/// What recovery will do for one step.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ts_rs::TS)]
#[serde(tag = "action", rename_all = "snake_case")]
#[ts(export, export_to = "recovery/")]
pub enum RecoveryItem {
    /// Put back what was captured before the step ran.
    Restore { step: String, what: Vec<String> },
    /// Run the step's approved recovery commands.
    Commands { step: String, commands: Vec<String> },
    /// The step cannot be undone, or said nothing about how.
    Cannot { step: String, reason: String },
}

impl RecoveryItem {
    pub fn step(&self) -> &str {
        match self {
            Self::Restore { step, .. } | Self::Commands { step, .. } | Self::Cannot { step, .. } => step,
        }
    }
}

fn describe(c: &Captured) -> String {
    match c {
        Captured::File { path, existed: true, .. } => format!("restore {path} from its backup"),
        Captured::File { path, existed: false, .. } => format!("remove {path}, which did not exist before"),
        Captured::RegistryValue { target, existed: true, value_kind, .. } => {
            format!("set {target} back to its earlier {} value", value_kind.as_deref().unwrap_or("?"))
        }
        Captured::RegistryValue { target, existed: false, .. } => {
            format!("remove {target}, which did not exist before")
        }
        Captured::ServiceState { name, exists: true, status, start_type } => format!(
            "set service {name} back to {} and {}",
            start_type.as_deref().unwrap_or("?"),
            status.as_deref().unwrap_or("?")
        ),
        Captured::ServiceState { name, exists: false, .. } => {
            format!("service {name} did not exist before; KeyJutsu will not delete a service")
        }
        Captured::PackageVersion { name } => format!("package {name}: versions cannot be restored yet"),
    }
}

/// The recovery plan for a stopped run: every step that ran or was running,
/// latest first, limited to `only` when given.
pub fn plan_recovery(
    snapshot: &ApprovedSnapshot,
    checkpoint: &Checkpoint,
    only: &[String],
) -> Result<Vec<RecoveryItem>, String> {
    let hashes = snapshot.step_hashes();
    let mut ran: Vec<(String, String)> =
        checkpoint.runs.iter().map(|r| (r.step.clone(), r.step_hash.clone())).collect();
    if let Some(p) = &checkpoint.in_progress {
        ran.push((p.step.clone(), p.step_hash.clone()));
    }
    for id in only {
        if !ran.iter().any(|(s, _)| s == id) {
            return Err(format!("step `{id}` did not run, so there is nothing to recover"));
        }
    }
    let mut items = Vec::new();
    for (id, ran_hash) in ran.iter().rev() {
        if !only.is_empty() && !only.contains(id) {
            continue;
        }
        let Some(step) = snapshot.plan().step(id) else {
            return Err(format!("step `{id}` is not in this snapshot"));
        };
        if hashes.get(id) != Some(ran_hash) {
            return Err(format!(
                "step `{id}` has changed since it ran; recover with the snapshot it ran under"
            ));
        }
        let item = match step.recovery.as_ref().map(|r| r.strategy) {
            // For an Administrator step run through the broker, this is the
            // broker's account of what it captured, shown here; the broker
            // restores from its own copy.
            Some(RecoveryStrategy::RestoreCapturedState) => {
                match checkpoint.captures.iter().rev().find(|c| &c.step == id && &c.step_hash == ran_hash) {
                    Some(c) => RecoveryItem::Restore {
                        step: id.clone(),
                        what: c.items.iter().map(describe).collect(),
                    },
                    None => RecoveryItem::Cannot {
                        step: id.clone(),
                        reason: "nothing was captured before it ran".into(),
                    },
                }
            }
            Some(RecoveryStrategy::Commands) => RecoveryItem::Commands {
                step: id.clone(),
                commands: step
                    .recovery
                    .iter()
                    .flat_map(|r| r.commands.iter().map(|c| c.text.clone()))
                    .collect(),
            },
            Some(RecoveryStrategy::None) => RecoveryItem::Cannot {
                step: id.clone(),
                reason: step
                    .reversibility
                    .as_ref()
                    .and_then(|r| r.notes.clone())
                    .unwrap_or_else(|| "the plan says it cannot be undone".into()),
            },
            None => RecoveryItem::Cannot { step: id.clone(), reason: "the plan declares no recovery".into() },
        };
        items.push(item);
    }
    Ok(items)
}

/// Whether carrying out `items` needs the elevation broker: recovering an
/// Administrator step, with KeyJutsu itself unelevated.
pub fn needs_broker(snapshot: &ApprovedSnapshot, items: &[RecoveryItem]) -> bool {
    !crate::elevation::is_elevated()
        && items.iter().any(|i| {
            matches!(i, RecoveryItem::Commands { .. } | RecoveryItem::Restore { .. })
                && snapshot
                    .plan()
                    .step(i.step())
                    .is_some_and(|s| s.privilege == Some(Privilege::Administrator))
        })
}

/// The result of recovering one step.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "recovery/")]
pub struct RecoveryResult {
    pub step: String,
    pub recovered: bool,
    pub checks: Vec<CheckResult>,
}

fn restore_one(c: &Captured, dir: &Path) -> CheckResult {
    let done = |check: String, ok: bool, detail: String| CheckResult { check, passed: Some(ok), detail };
    match c {
        // Exactly the file named, never through a link or junction: for the
        // broker, following one would write or delete as Administrator
        // wherever a link planted since the capture points.
        Captured::File { path, existed, sha256, backup } => {
            if *existed {
                let (Some(backup), Some(want)) = (backup, sha256) else {
                    return done(format!("file {path}"), false, "no backup was recorded".into());
                };
                let bytes = match std::fs::read(dir.join(backup)) {
                    Ok(b) => b,
                    Err(e) => return done(format!("file {path}"), false, format!("backup unreadable: {e}")),
                };
                if &sha256_hex(&bytes) != want {
                    return done(
                        format!("file {path}"),
                        false,
                        "the backup has changed since it was taken; not used".into(),
                    );
                }
                if let Err(e) = crate::exact_file::write(path, &bytes) {
                    return done(format!("file {path}"), false, e.to_string());
                }
                let now =
                    crate::exact_file::read(path).ok().flatten().map(|b| sha256_hex(&b)).unwrap_or_default();
                done(format!("file {path}"), &now == want, format!("sha256 {now}"))
            } else {
                if let Err(e) = crate::exact_file::remove(path) {
                    return done(format!("file {path}"), false, e.to_string());
                }
                let gone = matches!(crate::exact_file::read(path), Ok(None));
                done(format!("file {path}"), gone, "absent, as before".into())
            }
        }
        Captured::RegistryValue { target, existed, value_kind, value_json } => {
            let check = format!("registry {target}");
            let write = split_registry_target(target).and_then(|(key, name)| {
                pwsh_json(
                    &json!({"key": key, "name": name, "existed": existed, "kind": value_kind, "value": value_json}),
                    REGISTRY_WRITE,
                )
            });
            if let Err(e) = write {
                return done(check, false, e);
            }
            match read_registry(target) {
                Ok(now) => {
                    let same = now == (*existed, value_kind.clone(), value_json.clone());
                    done(check, same, if same { "as before".into() } else { format!("now {now:?}") })
                }
                Err(e) => done(check, false, e),
            }
        }
        Captured::ServiceState { name, exists: true, status, start_type } => {
            let check = format!("service {name}");
            if let Err(e) =
                pwsh_json(&json!({"name": name, "status": status, "start_type": start_type}), SERVICE_WRITE)
            {
                return done(check, false, e);
            }
            match read_service(name) {
                Ok((_, s, t)) => {
                    let same = s == *status && t == *start_type;
                    done(check, same, format!("{} and {}", t.unwrap_or_default(), s.unwrap_or_default()))
                }
                Err(e) => done(check, false, e),
            }
        }
        other => CheckResult { check: describe(other), passed: None, detail: "not restored".into() },
    }
}

/// Put back everything in one capture, checking each item.
pub fn restore_capture(capture: &StepCapture, dir: &Path) -> Vec<CheckResult> {
    capture.items.iter().map(|i| restore_one(i, dir)).collect()
}

/// Carry out a confirmed recovery plan, stopping at the first step whose
/// recovery fails: recovery is execution too, and does not carry on past a
/// failure. An Administrator step's recovery commands go to `elevated`, the
/// broker, when KeyJutsu itself is not elevated; they are never typed into
/// the unelevated shell. Recovery changes the machine, so the caller holds
/// the run lock, as for a run.
#[allow(clippy::too_many_arguments)]
pub fn recover(
    _held: &crate::runlock::RunLock,
    driver: Option<&Driver<'_>>,
    elevated: Option<&dyn crate::elevation::ElevatedRunner>,
    snapshot: &ApprovedSnapshot,
    checkpoint: &Checkpoint,
    dir: &Path,
    items: &[RecoveryItem],
    base: &PerformanceConfig,
    observe: &dyn Fn(&RecoveryResult),
) -> Vec<RecoveryResult> {
    let mut results = Vec::new();
    for item in items {
        let step = snapshot.plan().step(item.step());
        let mut checks = Vec::new();
        match item {
            RecoveryItem::Cannot { .. } => continue,
            // Captured by the broker, so restored by it, from its own copy.
            RecoveryItem::Restore { step: id, .. }
                if step.is_some_and(|s| s.privilege == Some(Privilege::Administrator))
                    && !crate::elevation::is_elevated() =>
            {
                let hash = snapshot.step_hashes().get(id).cloned().unwrap_or_default();
                match elevated.map(|r| r.restore_step(snapshot.snapshot_hash(), id, &hash)) {
                    Some(Ok(done)) => checks.extend(done),
                    Some(Err(e)) => checks.push(CheckResult {
                        check: "restore".into(),
                        passed: Some(false),
                        detail: format!("the elevation broker: {e}"),
                    }),
                    None => checks.push(CheckResult {
                        check: "restore".into(),
                        passed: Some(false),
                        detail: "it needs Administrator, and no elevation broker is running".into(),
                    }),
                }
            }
            RecoveryItem::Restore { step: id, .. } => {
                if let Some(c) = checkpoint.captures.iter().rev().find(|c| &c.step == id) {
                    checks.extend(c.items.iter().map(|i| restore_one(i, dir)));
                }
            }
            RecoveryItem::Commands { step: id, .. }
                if step.is_some_and(|s| s.privilege == Some(Privilege::Administrator))
                    && !crate::elevation::is_elevated() =>
            {
                let hash = snapshot.step_hashes().get(id).cloned().unwrap_or_default();
                let (ok, detail) = match elevated {
                    None => (false, "they need Administrator, and no elevation broker is running".to_owned()),
                    Some(runner) => match runner.recover_step(snapshot.snapshot_hash(), id, &hash) {
                        Ok(run) if !run.outcomes.iter().any(|o| matches!(o, StepOutcome::Failed { .. })) => {
                            (true, "ran as Administrator, through the elevation broker".to_owned())
                        }
                        Ok(_) => (false, "did not all succeed, through the elevation broker".to_owned()),
                        Err(e) => (false, format!("the elevation broker: {e}")),
                    },
                };
                checks.push(CheckResult { check: "recovery commands".into(), passed: Some(ok), detail });
            }
            RecoveryItem::Commands { step: id, commands } => {
                let Some(driver) = driver else {
                    checks.push(CheckResult {
                        check: "recovery commands".into(),
                        passed: Some(false),
                        detail: "no terminal to run them in".into(),
                    });
                    results.push(RecoveryResult { step: id.clone(), recovered: false, checks });
                    break;
                };
                let script = StagedScript {
                    steps: commands
                        .iter()
                        .enumerate()
                        .map(|(i, c)| StagedStep {
                            id: format!("{id}#r{i}"),
                            title: format!("Recover {id}"),
                            command: c.clone(),
                            mode: Some(EngineMode::Direct),
                            submit: None,
                            answers: None,
                        })
                        .collect(),
                };
                let config = PerformanceConfig { mode: EngineMode::Direct, ..base.clone() };
                let _ = driver.session.wait_for_prompt(Duration::from_secs(120));
                let ok = match perform(driver, script, config) {
                    Performed::Finished(o) => !o.iter().any(|o| matches!(o, StepOutcome::Failed { .. })),
                    Performed::Unfinished { .. } | Performed::Refused(_) => false,
                };
                checks.push(CheckResult {
                    check: "recovery commands".into(),
                    passed: Some(ok),
                    detail: if ok { "ran".into() } else { "did not all succeed".into() },
                });
            }
        }
        // The step's own recovery validation, whichever way it was undone.
        if let Some(r) = step.and_then(|s| s.recovery.as_ref()) {
            checks.extend(r.validation.iter().map(|c| run_check(c, None)));
        }
        let recovered = checks.iter().all(|c| c.passed != Some(false));
        let result = RecoveryResult { step: item.step().to_owned(), recovered, checks };
        observe(&result);
        results.push(result);
        if !recovered {
            break;
        }
    }
    results
}
