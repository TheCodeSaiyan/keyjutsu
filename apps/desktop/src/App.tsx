import { useCallback, useEffect, useRef, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import type {
  AdvanceStyle,
  ExecutionMode,
  KeyChord,
  PerformanceConfig,
  PerformanceSnapshot,
  ProfileMode,
  ReadinessReport,
  SessionEvent,
  ShellKind,
  StepOutcome,
  SubmitPolicy,
  TerminalMessage,
  TerminalProfile,
  TerminalSize,
} from "@keyjutsu/types";
import { INPUT_OWNED, ipc } from "./ipc";
import { TerminalView, type TerminalHandle } from "./components/TerminalView";
import { ReadinessPanel } from "./components/ReadinessPanel";
import { Overlay } from "./components/Overlay";
import mark from "./assets/mark.png";

const SHELL_NAMES: Record<ShellKind, string> = {
  pwsh: "PowerShell 7",
  windows_powershell: "Windows PowerShell 5.1",
  cmd: "Command Prompt",
};

const MODES: { value: ExecutionMode; label: string; hint: string }[] = [
  {
    value: "performance",
    label: "Performance",
    hint: "Each key you press types the next character.",
  },
  { value: "assisted", label: "Assisted", hint: "Each key types a short burst." },
  { value: "auto_performance", label: "Auto", hint: "Watch it type, run and check itself." },
  { value: "direct", label: "Direct", hint: "No typing effect: commands run as they are." },
];

const FIRST_RUN_KEY = "keyjutsu.firstRunSeen";

function describe(outcome: StepOutcome): string {
  switch (outcome.kind) {
    case "succeeded":
      return `succeeded (exit ${outcome.exit_code})`;
    case "failed":
      return `failed (exit ${outcome.exit_code})`;
    case "unverified":
      return "finished; success not verifiable in this shell";
  }
}

function summarise(s: PerformanceSnapshot | null): string | null {
  if (!s) return null;
  const last = s.outcomes.at(-1);
  if (s.state === "FAILED" && last)
    return `Step ${s.outcomes.length} ${describe(last)}. The performance stopped there.`;
  if (s.state === "COMPLETE") return `All ${s.step_count} steps finished.`;
  if (s.state === "ABORTED") return `Disarmed after ${s.outcomes.length} of ${s.step_count} steps.`;
  return null;
}

function readFlag(key: string): boolean {
  try {
    return localStorage.getItem(key) === "1";
  } catch {
    return false;
  }
}

export function App() {
  const [report, setReport] = useState<ReadinessReport | null>(null);
  const [profile, setProfile] = useState<TerminalProfile | null>(null);
  const [view, setView] = useState<"first-run" | "workspace">(() =>
    readFlag(FIRST_RUN_KEY) ? "workspace" : "first-run",
  );

  const [shell, setShell] = useState<ShellKind>("pwsh");
  const [shellProfile, setShellProfile] = useState<ProfileMode>("detected");
  const [generation, setGeneration] = useState(0);
  const [sessionId, setSessionId] = useState<number | null>(null);
  const [ready, setReady] = useState(false);
  const [exited, setExited] = useState(false);

  const [mode, setMode] = useState<ExecutionMode>("performance");
  const [advance, setAdvance] = useState<AdvanceStyle>("pure");
  const [submit, setSubmit] = useState<SubmitPolicy>("any_key");
  const [source, setSource] = useState<"demo" | "own">("demo");
  const [ownCommands, setOwnCommands] = useState("");

  const [armed, setArmed] = useState(false);
  const [snapshot, setSnapshot] = useState<PerformanceSnapshot | null>(null);
  const [overlay, setOverlay] = useState(false);
  const [paused, setPaused] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const term = useRef<TerminalHandle>(null);
  const snapshotRef = useRef<PerformanceSnapshot | null>(null);
  const titleRef = useRef("Windows PowerShell");
  const overlayRef = useRef(false);
  const sessionRef = useRef<number | null>(null);
  useEffect(() => {
    sessionRef.current = sessionId;
  }, [sessionId]);

  useEffect(() => {
    ipc
      .terminalProfile()
      .then(setProfile)
      .catch(() => setProfile(null));
    ipc
      .readinessScan()
      .then((r) => {
        setReport(r);
        if (!r.shells.some((s) => s.kind === "pwsh") && r.shells[0]) setShell(r.shells[0].kind);
      })
      .catch((e) => setError(String(e)));
  }, []);

  const onEvent = useCallback((event: SessionEvent) => {
    switch (event.type) {
      case "shell_ready":
        setReady(true);
        break;
      case "performance":
        snapshotRef.current = event.snapshot;
        setSnapshot(event.snapshot);
        break;
      case "released":
        setArmed(false);
        overlayRef.current = false;
        setOverlay(false);
        setPaused(false);
        setNotice(summarise(snapshotRef.current));
        break;
      case "overlay_requested":
        // Opening the overlay pauses staged input. Closing it with the chord
        // again does not resume: that takes the Resume button, so a stray
        // chord cannot restart a performance.
        if (!overlayRef.current && sessionRef.current !== null) {
          void ipc.pause(sessionRef.current);
          setPaused(true);
        }
        overlayRef.current = !overlayRef.current;
        setOverlay(overlayRef.current);
        break;
      case "exited":
        setExited(true);
        setReady(false);
        setArmed(false);
        break;
      default:
        break;
    }
  }, []);

  // Starting a different terminal: forget everything about the old one. Done
  // here, where the choice is made, rather than inside the effect below.
  const restart = (change: () => void) => {
    setReady(false);
    setExited(false);
    setSessionId(null);
    term.current?.reset();
    change();
  };

  // One live session for the chosen shell and profile.
  useEffect(() => {
    if (view !== "workspace") return;
    let id: number | null = null;
    let cancelled = false;
    const size = term.current?.size() ?? { rows: 30, cols: 120 };
    ipc
      .openTerminal({ shell, profile: shellProfile, size }, (m: TerminalMessage) => {
        if (m.kind === "output") term.current?.write(m.data);
        else onEvent(m.event);
      })
      .then((opened) => {
        id = opened;
        if (cancelled) void ipc.close(opened);
        else setSessionId(opened);
      })
      .catch((e) => setError(String(e)));
    return () => {
      cancelled = true;
      if (id !== null) void ipc.close(id);
    };
  }, [view, shell, shellProfile, generation, onEvent]);

  // The window title follows the shell's own title while armed, as the user's
  // terminal would, and says KeyJutsu otherwise.
  useEffect(() => {
    void getCurrentWindow()
      .setTitle(armed ? titleRef.current : "KeyJutsu")
      .catch(() => undefined);
  }, [armed]);

  const config: PerformanceConfig = {
    mode,
    advance,
    submit,
    cadence: { base_ms: 55, variance_ms: 30, punctuation_pause_ms: 110, boundary_pause_ms: 550 },
    seed: 0,
    hold_input_after_complete: mode !== "direct",
  };

  const arm = async () => {
    if (sessionId === null) return;
    setError(null);
    setNotice(null);
    const commands = ownCommands.split(/\r?\n/).filter((l) => l.trim() !== "");
    try {
      const s = await ipc.arm(
        sessionId,
        source === "demo" ? { kind: "safe_demo" } : { kind: "operator_commands", commands },
        config,
      );
      snapshotRef.current = s;
      setSnapshot(s);
      setArmed(true);
    } catch (e) {
      setError(String(e));
    }
  };

  const onData = useCallback(
    (data: string) => {
      if (sessionId === null) return;
      ipc.write(sessionId, data).catch((e) => {
        if (String(e) !== INPUT_OWNED) setError(String(e));
      });
    },
    [sessionId],
  );
  const onKey = useCallback(
    (chord: KeyChord) => {
      if (sessionId !== null) void ipc.key(sessionId, chord);
    },
    [sessionId],
  );
  const onResize = useCallback(
    (size: TerminalSize) => {
      if (sessionId !== null) void ipc.resize(sessionId, size);
    },
    [sessionId],
  );

  if (view === "first-run") {
    return (
      <main className="first-run">
        <h1 className="brand-heading">
          <img src={mark} alt="" width={44} height={44} />
          KeyJutsu
        </h1>
        <p className="lede">
          Checking this machine before anything runs. Nothing is changed: each shell is started once
          in a throwaway terminal and asked to print a word.
        </p>
        <ReadinessPanel report={report} />
        {error && (
          <p role="alert" className="error">
            {error}
          </p>
        )}
        <div className="row">
          <button
            className="primary"
            onClick={() => {
              try {
                localStorage.setItem(FIRST_RUN_KEY, "1");
              } catch {
                /* the choice just is not remembered */
              }
              setView("workspace");
            }}
          >
            Start using KeyJutsu
          </button>
          <button
            onClick={() => {
              setSource("demo");
              setView("workspace");
            }}
          >
            Try the safe demo
          </button>
        </div>
      </main>
    );
  }

  const canArm = sessionId !== null && ready && !exited && !armed;
  const modeHint = MODES.find((m) => m.value === mode)?.hint;

  return (
    <div className="app" data-armed={armed}>
      {!armed && (
        <header className="topbar">
          <span className="wordmark">
            <img src={mark} alt="" width={22} height={22} />
            KeyJutsu
          </span>
          <label>
            Shell{" "}
            <select
              value={shell}
              onChange={(e) => restart(() => setShell(e.target.value as ShellKind))}
            >
              {(report?.shells ?? [{ kind: "pwsh" as ShellKind }]).map((s) => (
                <option key={s.kind} value={s.kind}>
                  {SHELL_NAMES[s.kind]}
                </option>
              ))}
            </select>
          </label>
          <label>
            Profile{" "}
            <select
              value={shellProfile}
              onChange={(e) => restart(() => setShellProfile(e.target.value as ProfileMode))}
            >
              <option value="detected">Mine</option>
              <option value="clean">Clean</option>
            </select>
          </label>
          <button onClick={() => restart(() => setGeneration((g) => g + 1))}>New terminal</button>
        </header>
      )}

      <div className="body">
        {!armed && (
          <aside className="side" aria-label="Performance">
            <section>
              <h2>Performance</h2>
              <fieldset>
                <legend>Mode</legend>
                {MODES.map((m) => (
                  <label key={m.value} className="choice">
                    <input
                      type="radio"
                      name="mode"
                      checked={mode === m.value}
                      onChange={() => setMode(m.value)}
                    />
                    {m.label}
                  </label>
                ))}
                <p className="muted small">{modeHint}</p>
              </fieldset>
              {mode === "performance" && (
                <label className="choice">
                  <input
                    type="checkbox"
                    checked={advance === "turbo"}
                    onChange={(e) => setAdvance(e.target.checked ? "turbo" : "pure")}
                  />
                  Turbo: a word per key
                </label>
              )}
              {(mode === "performance" || mode === "assisted") && (
                <label>
                  Submitting{" "}
                  <select
                    value={submit}
                    onChange={(e) => setSubmit(e.target.value as SubmitPolicy)}
                  >
                    <option value="any_key">Any key sends Enter</option>
                    <option value="require_enter">Only a real Enter</option>
                    <option value="auto_submit">Submit automatically</option>
                  </select>
                </label>
              )}
              <fieldset>
                <legend>Commands</legend>
                <label className="choice">
                  <input
                    type="radio"
                    name="source"
                    checked={source === "demo"}
                    onChange={() => setSource("demo")}
                  />
                  Safe demo (read-only)
                </label>
                <label className="choice">
                  <input
                    type="radio"
                    name="source"
                    checked={source === "own"}
                    onChange={() => setSource("own")}
                  />
                  My own commands
                </label>
                {source === "own" && (
                  <>
                    <textarea
                      aria-label="Commands, one per line"
                      rows={5}
                      spellCheck={false}
                      value={ownCommands}
                      onChange={(e) => setOwnCommands(e.target.value)}
                      placeholder={"Get-Service -Name Winmgmt\nGet-Date"}
                    />
                    <p className="muted small">
                      These run exactly as if you typed them. They are not validated or approved:
                      plans and approval arrive in later milestones.
                    </p>
                  </>
                )}
              </fieldset>
              <button className="primary arm" disabled={!canArm} onClick={arm}>
                Arm KeyJutsu
              </button>
              <p className="muted small">
                {exited
                  ? "The shell has exited. Open a new terminal to continue."
                  : !ready
                    ? "Waiting for the shell's first prompt…"
                    : "Arming hands the keyboard to the performance. Ctrl+Alt+Shift+K disarms; Ctrl+Shift+K opens the controls."}
              </p>
              {notice && (
                <p role="status" className="notice">
                  {notice}
                </p>
              )}
              {error && (
                <p role="alert" className="error">
                  {error}
                </p>
              )}
            </section>
            <section>
              <h2>Readiness</h2>
              <ReadinessPanel report={report} />
            </section>
          </aside>
        )}

        <section className="term-area">
          <TerminalView
            ref={term}
            profile={profile}
            armed={armed}
            onData={onData}
            onKey={onKey}
            onResize={onResize}
            onTitle={(t) => (titleRef.current = t)}
          />
          {armed && overlay && (
            <Overlay
              snapshot={snapshot}
              paused={paused}
              onResume={() => {
                if (sessionId !== null) void ipc.resume(sessionId);
                setPaused(false);
                overlayRef.current = false;
                setOverlay(false);
                term.current?.focus();
              }}
              onDisarm={() => {
                if (sessionId !== null) void ipc.disarm(sessionId);
              }}
            />
          )}
        </section>
      </div>

      <div className="visually-hidden" aria-live="polite">
        {armed && snapshot
          ? `Armed. Step ${snapshot.step_index + 1} of ${snapshot.step_count}.`
          : ""}
      </div>
    </div>
  );
}
