//! KeyJutsu's elevation broker (§26, ADR 0011).
//!
//! The broker is started elevated, once, before a plan runs, pinned to one
//! approved snapshot by its hash. It accepts exactly one kind of work: *run
//! approved step X of snapshot S, whose hash is H*. It recomputes everything
//! from its own verified copy of the snapshot and refuses on any difference.
//! There is no request that carries a command: an altered command changes
//! the step's hash, and the broker refuses it.
//!
//! Its pipe admits only the launching Windows account, rejects remote
//! clients and cannot be pre-created by someone else; the connecting process
//! must be the one that launched the broker, and must know the per-launch
//! secret. The protocol is versioned, and a different version is refused,
//! not negotiated.

use std::io::{Read, Write};
use std::path::Path;
use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use keyjutsu_core::elevation::{ElevatedRun, ElevatedRunner};
use keyjutsu_core::execute::{ForwardingSink, Performed, perform, staged_for};
use keyjutsu_core::execution::{ExecutionMode, PerformanceConfig, StepOutcome};
use keyjutsu_core::headless::Collector;
use keyjutsu_core::plan::ApprovedSnapshot;
use keyjutsu_core::plan::model::{Privilege, ShellName, Step, ValidationDisplay};
use keyjutsu_core::terminal::{ProfileMode, ShellKind};
use keyjutsu_core::{Session, SessionOptions};
use serde::{Deserialize, Serialize};

#[cfg(windows)]
pub mod pipe;

/// Both ends must speak exactly this version.
pub const PROTOCOL: u32 = 1;
/// No message is larger than this.
const MAX_FRAME: usize = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    Hello { protocol: u32, secret: String },
    RunStep { snapshot_hash: String, step: String, step_hash: String },
    Goodbye,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Response {
    Welcome { protocol: u32, snapshot_hash: String, elevated: bool },
    Refused { reason: String },
    StepDone { outcomes: Vec<StepOutcome>, output: String },
    Bye,
}

pub fn write_frame(w: &mut impl Write, bytes: &[u8]) -> std::io::Result<()> {
    let len = u32::try_from(bytes.len()).map_err(|_| std::io::Error::other("frame too large"))?;
    w.write_all(&len.to_le_bytes())?;
    w.write_all(bytes)?;
    w.flush()
}

/// `None` when the other side has gone.
pub fn read_frame(r: &mut impl Read) -> std::io::Result<Option<Vec<u8>>> {
    let mut len = [0u8; 4];
    let mut got = 0;
    while got < 4 {
        let n = r.read(&mut len[got..])?;
        if n == 0 {
            return Ok(None);
        }
        got += n;
    }
    let len = u32::from_le_bytes(len) as usize;
    if len > MAX_FRAME {
        return Err(std::io::Error::other("frame too large"));
    }
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf)?;
    Ok(Some(buf))
}

/// Compare secrets without stopping at the first difference.
fn same_secret(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Runs a step, elevated in the real broker, plainly in tests.
pub type StepRunner = Box<dyn FnMut(&ApprovedSnapshot, &Step) -> Result<ElevatedRun, String> + Send>;

pub struct Broker {
    snapshot: ApprovedSnapshot,
    secret: String,
    authenticated: bool,
    run: StepRunner,
}

impl std::fmt::Debug for Broker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Broker").field("snapshot", &self.snapshot.snapshot_hash()).finish_non_exhaustive()
    }
}

impl Broker {
    pub fn new(snapshot: ApprovedSnapshot, secret: String, run: StepRunner) -> Self {
        Self { snapshot, secret, authenticated: false, run }
    }

    /// Answer one request. Anything not understood is refused: there is no
    /// request that could carry a command.
    pub fn handle(&mut self, raw: &[u8]) -> Response {
        let refuse = |reason: String| Response::Refused { reason };
        let request: Request = match serde_json::from_slice(raw) {
            Ok(r) => r,
            Err(_) => return refuse("not a request this broker accepts".into()),
        };
        match request {
            Request::Hello { protocol, secret } => {
                if protocol != PROTOCOL {
                    return refuse(format!(
                        "protocol version {protocol} is not this broker's {PROTOCOL}; KeyJutsu and its broker must be the same release"
                    ));
                }
                if !same_secret(&secret, &self.secret) {
                    return refuse("not authenticated".into());
                }
                self.authenticated = true;
                Response::Welcome {
                    protocol: PROTOCOL,
                    snapshot_hash: self.snapshot.snapshot_hash().to_owned(),
                    elevated: keyjutsu_core::elevation::is_elevated(),
                }
            }
            Request::Goodbye => Response::Bye,
            Request::RunStep { .. } if !self.authenticated => refuse("not authenticated".into()),
            Request::RunStep { snapshot_hash, step, step_hash } => {
                if snapshot_hash != self.snapshot.snapshot_hash() {
                    return refuse(format!(
                        "unknown plan {snapshot_hash}: this broker was started for {}",
                        self.snapshot.snapshot_hash()
                    ));
                }
                let Some(s) = self.snapshot.plan().step(&step).cloned() else {
                    return refuse(format!("there is no step `{step}` in the approved plan"));
                };
                if s.privilege != Some(Privilege::Administrator) {
                    return refuse(format!(
                        "step `{step}` does not need Administrator, and the broker runs only steps that do"
                    ));
                }
                let approved = self.snapshot.step_hashes().get(&step).cloned().unwrap_or_default();
                if step_hash != approved {
                    return refuse(format!(
                        "step `{step}` is not the approved version: its hash differs, so its command or what it depends on was altered"
                    ));
                }
                match (self.run)(&self.snapshot, &s) {
                    Ok(run) => Response::StepDone { outcomes: run.outcomes, output: run.output },
                    Err(e) => refuse(format!("step `{step}` could not be run: {e}")),
                }
            }
        }
    }
}

/// Run a step's lines in a fresh shell of its own, directly (no typing
/// performance), and report each line's outcome and what was printed.
/// In the broker this shell is elevated because the broker is.
pub fn run_in_shell(snapshot: &ApprovedSnapshot, step: &Step) -> Result<ElevatedRun, String> {
    let kind = match step.shell.as_ref().map(|s| s.kind) {
        Some(ShellName::WindowsPowershell) => ShellKind::WindowsPowershell,
        Some(ShellName::Cmd) => ShellKind::Cmd,
        _ => ShellKind::Pwsh,
    };
    let (tx, events) = channel();
    let out = Arc::new(Collector::new());
    let sink = Arc::new(ForwardingSink { inner: out.clone(), events: Mutex::new(tx) });
    let mut o = SessionOptions::new(kind);
    o.profile = ProfileMode::Clean;
    o.intercept_cursor_queries = true;
    let session = Session::open(o, sink).map_err(|e| e.to_string())?;
    if !session.wait_ready(Duration::from_secs(60)) {
        session.close();
        return Err("the elevated shell never showed a prompt".into());
    }
    let show = snapshot.plan().execution_preferences.as_ref().and_then(|p| p.show_validation)
        != Some(ValidationDisplay::None);
    let before = out.plain_output().len();
    let config = PerformanceConfig { mode: ExecutionMode::Direct, ..PerformanceConfig::default() };
    let driver = keyjutsu_core::execute::Driver { session: &session, events: &events };
    let performed = perform(&driver, staged_for(step, show), config);
    let _ = session.wait_for_prompt(Duration::from_secs(10));
    let text = out.plain_output();
    session.close();
    let output = text.get(before..).unwrap_or("").to_owned();
    match performed {
        Performed::Finished(outcomes) => Ok(ElevatedRun { outcomes, output }),
        Performed::Refused(r) => Err(r),
        Performed::Unfinished { .. } => Err("the elevated shell ended before the step finished".into()),
    }
}

/// Accept one connection, and only from `expected_pid`: the process that
/// launched the broker.
#[cfg(windows)]
pub fn accept_launcher(server: &pipe::ServerPipe, expected_pid: u32) -> Result<(), String> {
    let pid = server.accept()?;
    if pid != expected_pid {
        return Err(format!("refused process {pid}: only the launching process {expected_pid} may connect"));
    }
    Ok(())
}

/// Exit codes of `keyjutsu-broker`, so a refusal says why.
pub mod exit {
    pub const USAGE: u8 = 2;
    pub const SNAPSHOT_UNREADABLE: u8 = 4;
    pub const SNAPSHOT_NOT_VERIFIED: u8 = 5;
    pub const SNAPSHOT_NOT_LAUNCHED: u8 = 6;
    pub const PIPE: u8 = 7;
    pub const WRONG_CLIENT: u8 = 8;
    pub const NOBODY_CAME: u8 = 9;
}

/// Serve one client on `stream` until it says goodbye or goes away.
pub fn serve(stream: &mut (impl Read + Write), broker: &mut Broker) -> std::io::Result<()> {
    while let Some(frame) = read_frame(stream)? {
        let response = broker.handle(&frame);
        let bye = response == Response::Bye;
        write_frame(stream, &serde_json::to_vec(&response).map_err(std::io::Error::other)?)?;
        if bye {
            break;
        }
    }
    Ok(())
}

/// KeyJutsu's end of the pipe.
#[derive(Debug)]
pub struct BrokerClient {
    stream: Mutex<std::fs::File>,
    /// The broker reported running elevated.
    pub elevated: bool,
}

impl BrokerClient {
    /// Connect to `\\.\pipe\<name>`, waiting up to `wait` for the broker to
    /// create it, and introduce ourselves.
    pub fn connect(name: &str, secret: &str, wait: Duration) -> Result<Self, String> {
        let path = format!(r"\\.\pipe\{name}");
        let deadline = std::time::Instant::now() + wait;
        let file = loop {
            match std::fs::OpenOptions::new().read(true).write(true).open(&path) {
                Ok(f) => break f,
                Err(e) if std::time::Instant::now() >= deadline => {
                    return Err(format!("the elevation broker did not start: {e}"));
                }
                Err(_) => std::thread::sleep(Duration::from_millis(100)),
            }
        };
        let mut client = Self { stream: Mutex::new(file), elevated: false };
        match client.ask(&Request::Hello { protocol: PROTOCOL, secret: secret.to_owned() })? {
            Response::Welcome { elevated, .. } => {
                client.elevated = elevated;
                Ok(client)
            }
            Response::Refused { reason } => Err(reason),
            other => Err(format!("unexpected answer: {other:?}")),
        }
    }

    pub fn ask(&self, request: &Request) -> Result<Response, String> {
        let mut stream = self.stream.lock().map_err(|_| "the broker connection is broken")?;
        let bytes = serde_json::to_vec(request).map_err(|e| e.to_string())?;
        write_frame(&mut *stream, &bytes).map_err(|e| e.to_string())?;
        let frame = read_frame(&mut *stream).map_err(|e| e.to_string())?.ok_or("the broker went away")?;
        serde_json::from_slice(&frame).map_err(|e| e.to_string())
    }

    /// Send raw bytes, for tests that the broker refuses what it does not know.
    pub fn ask_raw(&self, bytes: &[u8]) -> Result<Response, String> {
        let mut stream = self.stream.lock().map_err(|_| "the broker connection is broken")?;
        write_frame(&mut *stream, bytes).map_err(|e| e.to_string())?;
        let frame = read_frame(&mut *stream).map_err(|e| e.to_string())?.ok_or("the broker went away")?;
        serde_json::from_slice(&frame).map_err(|e| e.to_string())
    }
}

impl ElevatedRunner for BrokerClient {
    fn run_step(&self, snapshot_hash: &str, step: &str, step_hash: &str) -> Result<ElevatedRun, String> {
        match self.ask(&Request::RunStep {
            snapshot_hash: snapshot_hash.to_owned(),
            step: step.to_owned(),
            step_hash: step_hash.to_owned(),
        })? {
            Response::StepDone { outcomes, output } => Ok(ElevatedRun { outcomes, output }),
            Response::Refused { reason } => Err(reason),
            other => Err(format!("unexpected answer: {other:?}")),
        }
    }
}

impl Drop for BrokerClient {
    fn drop(&mut self) {
        let _ = self.ask(&Request::Goodbye);
    }
}

/// 32 random bytes as hex: a pipe name or a secret.
pub fn random_hex() -> String {
    let mut b = [0u8; 32];
    let _ = getrandom::fill(&mut b);
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Start the broker elevated (one UAC prompt) for `snapshot`, and connect.
pub fn launch(broker_exe: &Path, snapshot_file: &Path, snapshot_hash: &str) -> Result<BrokerClient, String> {
    let name = format!("keyjutsu-broker-{}", &random_hex()[..32]);
    let secret = random_hex();
    let args = [
        "--pipe".to_owned(),
        name.clone(),
        "--client-pid".into(),
        std::process::id().to_string(),
        "--snapshot".into(),
        snapshot_file.display().to_string(),
        "--snapshot-hash".into(),
        snapshot_hash.to_owned(),
        "--secret".into(),
        secret.clone(),
    ];
    let quoted: Vec<String> = args.iter().map(|a| format!("'{}'", a.replace('\'', "''"))).collect();
    let script = format!(
        "Start-Process -FilePath '{}' -ArgumentList @({}) -Verb RunAs -WindowStyle Hidden",
        broker_exe.display().to_string().replace('\'', "''"),
        quoted.join(",")
    );
    let status = std::process::Command::new("powershell.exe")
        .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-Command", &script])
        .status()
        .map_err(|e| e.to_string())?;
    if !status.success() {
        return Err("elevation was not granted".into());
    }
    BrokerClient::connect(&name, &secret, Duration::from_secs(60))
}

/// Where the broker binary sits: next to the running program.
pub fn broker_path() -> Option<std::path::PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let p = exe.with_file_name("keyjutsu-broker.exe");
    p.exists().then_some(p)
}
