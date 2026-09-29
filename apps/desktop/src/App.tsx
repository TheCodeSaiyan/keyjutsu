import { useCallback, useEffect, useRef, useState } from "react";
import { getCurrentWindow, UserAttentionType } from "@tauri-apps/api/window";
import type {
  agent,
  execute,
  workspace,
  RunMessage,
  Sealed,
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
  WaitingRun,
} from "@keyjutsu/types";
import { INPUT_OWNED, ipc } from "./ipc";
import { TerminalView, type TerminalHandle } from "./components/TerminalView";
import { ReadinessPanel } from "./components/ReadinessPanel";
import { DiagnosticsPanel } from "./components/DiagnosticsPanel";
import { UpdatePanel } from "./components/UpdatePanel";
import { Overlay } from "./components/Overlay";
import { NewTask } from "./components/NewTask";
import { PlanWorkspace } from "./components/PlanWorkspace";
import { CriticalDialog } from "./components/CriticalDialog";
import { RunPanel } from "./components/RunPanel";
import { ExportRecording } from "./components/ExportRecording";
import { ResumeDialog } from "./components/ResumeDialog";
import { HistoryView } from "./components/HistoryView";
import { TechniquesView } from "./components/TechniquesView";
import { boundaryName, confirmationFor } from "./plan";
import { stagedError } from "./staging";
import {
  PRESENTATIONS,
  isOperatorChord,
  loadPresentation,
  rules,
  savePresentation,
  type Presentation,
} from "./presentation";
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
  // Where the operator is: the task, the plan, or the terminal.
  const [space, setSpace] = useState<"task" | "plan" | "terminal" | "history" | "techniques">(
    "task",
  );
  const [agents, setAgents] = useState<agent.AgentInfo[] | null>(null);
  const [ws, setWs] = useState<workspace.WorkspaceView | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [sealed, setSealed] = useState<Sealed | null>(null);
  // A run stopped at a restart or other boundary, and what was checked
  // before crossing it.
  const [waiting, setWaiting] = useState<WaitingRun | null>(null);
  const [resumeNotice, setResumeNotice] = useState<execute.BoundaryNotice | null>(null);
  // How KeyJutsu asks for the operator mid-performance, and what it is
  // waiting on: a question not yet opened (unless the choice is to show it
  // at once), a credential to type, or the end of a run held on screen.
  const [presentation, setPresentation] = useState<Presentation>(loadPresentation);
  const [revealed, setRevealed] = useState(false);
  const [credential, setCredential] = useState<string | null>(null);
  const [holding, setHolding] = useState(false);
  // A staged error is on screen; where it would not fit, the edge instead.
  const [staged, setStaged] = useState<boolean | null>(null);
  const presentationRef = useRef(presentation);
  useEffect(() => {
    presentationRef.current = presentation;
    savePresentation(presentation);
  }, [presentation]);
  const [running, setRunning] = useState(false);
  const [runDone, setRunDone] = useState<Extract<RunMessage, { kind: "done" }> | null>(null);
  const [critical, setCritical] = useState<{
    confirmation: execute.CriticalConfirmation;
    purpose: "approve" | "run";
    rest: string[];
    typed: Record<string, string>;
  } | null>(null);

  // Chosen once the readiness scan says which shells exist; opening PowerShell 7
  // before then failed on machines without it and left the error on screen.
  const [shell, setShell] = useState<ShellKind | null>(null);
  const [shellProfile, setShellProfile] = useState<ProfileMode>("detected");
  const [generation, setGeneration] = useState(0);
  const [sessionId, setSessionId] = useState<number | null>(null);
  const [ready, setReady] = useState(false);
  const [exited, setExited] = useState(false);

  const [mode, setMode] = useState<ExecutionMode>("performance");
  // Record the next run (ADR 0021), and whether the last one was.
  const [record, setRecord] = useState(false);
  const [recorded, setRecorded] = useState(false);
  const [exporting, setExporting] = useState<string | null>(null);
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

  // The plan workspace: agents, and a plan already open (from a file given
  // at start, for instance).
  useEffect(() => {
    ipc
      .agents()
      .then(setAgents)
      .catch(() => setAgents([]));
    // After a restart, a plan may be waiting on this side of it. That is
    // not a first run, whatever the first-run marker says: go straight to
    // the offer to continue it.
    ipc
      .waitingRun()
      .then((w) => {
        setWaiting(w);
        if (w) setView("workspace");
      })
      .catch(() => setWaiting(null));
    ipc
      .workspace()
      .then((w) => {
        // A plan opened at start goes straight to the workspace.
        if (w) {
          setWs(w);
          setSpace("plan");
          setView("workspace");
        }
      })
      .catch(() => undefined);
  }, []);

  useEffect(() => {
    ipc
      .terminalProfile()
      .then(setProfile)
      .catch(() => setProfile(null));
    ipc
      .readinessScan()
      .then((r) => {
        setReport(r);
        setShell(
          r.shells.some((s) => s.kind === "pwsh")
            ? "pwsh"
            : (r.shells[0]?.kind ?? "windows_powershell"),
        );
      })
      .catch((e) => {
        setError(String(e));
        setShell("windows_powershell");
      });
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
        if (rules(presentationRef.current).holdAfterRun) setHolding(true);
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
    if (view !== "workspace" || shell === null) return;
    let id: number | null = null;
    let cancelled = false;
    const size = term.current?.size() ?? { rows: 30, cols: 120 };
    ipc
      .openTerminal({ shell, profile: shellProfile, size }, (m: TerminalMessage) => {
        // A replaced session still reports its own exit after the new one
        // has opened; heard, it marked the new terminal as exited.
        if (cancelled) return;
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

  /** A workspace request: show what is happening, then the new view. */
  const act = (label: string, request: () => Promise<workspace.WorkspaceView>) => {
    setBusy(label);
    setError(null);
    request()
      .then((w) => {
        setWs(w);
        setSealed(null);
        setSpace("plan");
      })
      .catch((e) => setError(String(e)))
      .finally(() => setBusy(null));
  };

  const approve = async (typed: Record<string, string>) => {
    setBusy("Approving and sealing…");
    setError(null);
    try {
      setSealed(await ipc.approve(typed));
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(null);
    }
  };

  // Approval asks for each critical step's phrase in turn, then seals.
  const startApproval = () => {
    if (!ws) return;
    const criticals = ws.steps.filter((s) => s.critical).map((s) => s.id);
    const first = criticals[0];
    const c = first ? confirmationFor(ws, first) : null;
    if (!c) {
      void approve({});
      return;
    }
    setCritical({ confirmation: c, purpose: "approve", rest: criticals.slice(1), typed: {} });
  };

  const onRun = useCallback((m: RunMessage) => {
    switch (m.kind) {
      case "confirm":
        setCritical({ confirmation: m.confirmation, purpose: "run", rest: [], typed: {} });
        break;
      case "resume":
        setResumeNotice(m.notice);
        break;
      case "done":
        setRunning(false);
        setRunDone(m);
        setWaiting(null);
        setCredential(null);
        if (rules(presentationRef.current).holdAfterRun) setHolding(true);
        break;
      case "execution":
        // An Administrator step ran in the elevation broker's shell: show
        // what it printed in the terminal.
        if (m.event.kind === "elevated_output")
          term.current?.write(m.event.text.replace(/\r?\n/g, "\r\n"));
        if (m.event.kind === "credential_required") {
          setNotice(`Credential required: ${m.event.prompt}. Stop typing, then press Enter.`);
          setCredential(m.event.prompt);
        }
        if (m.event.kind === "step_finished" || m.event.kind === "step_starting")
          setCredential(null);
        break;
    }
  }, []);

  const armPlan = async () => {
    if (sessionId === null) return;
    setError(null);
    setNotice(null);
    setRunDone(null);
    setSpace("terminal");
    setRunning(true);
    try {
      setRecorded(record);
      await ipc.runPlan(sessionId, config, record, onRun);
      term.current?.focus();
    } catch (e) {
      setRunning(false);
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
      if (holding) {
        // The run is over but the stage is held: nothing reaches the real
        // shell until the operator lets go with the chord.
        if (isOperatorChord(chord)) setHolding(false);
        return;
      }
      const unopened =
        !revealed && ((critical && critical.purpose === "run") || resumeNotice !== null);
      if (unopened && isOperatorChord(chord)) {
        setRevealed(true);
        return;
      }
      if (sessionId !== null) void ipc.key(sessionId, chord);
    },
    [sessionId, holding, revealed, critical, resumeNotice],
  );
  // A question arrived: with "Stage an error", show one. Taken away again,
  // synchronously, before the answer lets anything more reach the terminal.
  const waitingFor =
    critical !== null && critical.purpose === "run" ? "confirm" : resumeNotice ? "resume" : null;
  useEffect(() => {
    if (!waitingFor || !rules(presentation).stagedError || staged !== null || !term.current) return;
    const lines = stagedError(waitingFor, shell ?? "pwsh", term.current.promptText());
    setStaged(term.current.stage(lines));
  }, [waitingFor, presentation, staged, shell]);
  const unstage = () => {
    term.current?.unstage();
    setStaged(null);
  };

  // KeyJutsu is waiting on the operator. Out of view, the taskbar button
  // flashes, which a shared window does not show.
  const waitingNow =
    (critical !== null && critical.purpose === "run") ||
    resumeNotice !== null ||
    credential !== null ||
    holding;
  useEffect(() => {
    if (waitingNow && rules(presentation).flashTaskbar) {
      getCurrentWindow()
        .requestUserAttention(UserAttentionType.Informational)
        .catch(() => undefined);
    }
  }, [waitingNow, presentation]);

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
  // During a performance, and for the whole of a plan run, only the terminal
  // is on screen.
  const fullTerminal = armed || running || holding;
  const r = rules(presentation);
  // Shown at once when the operator asked for that, otherwise once opened.
  const showQuestion = r.coverTerminal || revealed;
  const waitingOnOperator = waitingNow;

  const dialog = critical && (critical.purpose === "approve" || showQuestion) && (
    <CriticalDialog
      discreet={critical.purpose === "run" && !r.coverTerminal}
      key={critical.confirmation.step + critical.purpose}
      confirmation={critical.confirmation}
      purpose={critical.purpose}
      onConfirm={(typed) => {
        if (critical.purpose === "run") {
          unstage();
          setCritical(null);
          setRevealed(false);
          void ipc.confirm(typed);
          term.current?.focus();
          return;
        }
        const collected = { ...critical.typed, [critical.confirmation.step]: typed };
        const [next, ...rest] = critical.rest;
        const c = next && ws ? confirmationFor(ws, next) : null;
        if (c) setCritical({ confirmation: c, purpose: "approve", rest, typed: collected });
        else {
          setCritical(null);
          void approve(collected);
        }
      }}
      onCancel={() => {
        if (critical.purpose === "run") {
          unstage();
          void ipc.confirm(null);
        }
        setCritical(null);
        setRevealed(false);
      }}
    />
  );

  return (
    <div className="shell" data-full={fullTerminal}>
      {!fullTerminal && (
        <nav className="rail" aria-label="KeyJutsu">
          <span className="wordmark">
            <img src={mark} alt="" width={28} height={28} />
            <span>
              Key<span className="accent-j">J</span>utsu
            </span>
          </span>
          <ul>
            <li>
              <button
                aria-current={space === "task" ? "page" : undefined}
                onClick={() => setSpace("task")}
              >
                New task
              </button>
            </li>
            <li>
              <button
                aria-current={space === "plan" ? "page" : undefined}
                disabled={!ws}
                onClick={() => setSpace("plan")}
              >
                Plan
              </button>
            </li>
            <li>
              <button
                aria-current={space === "terminal" ? "page" : undefined}
                onClick={() => setSpace("terminal")}
              >
                Terminal
              </button>
            </li>
            <li>
              <button
                aria-current={space === "history" ? "page" : undefined}
                onClick={() => setSpace("history")}
              >
                History
              </button>
            </li>
            <li>
              <button
                aria-current={space === "techniques" ? "page" : undefined}
                onClick={() => setSpace("techniques")}
              >
                Techniques
              </button>
            </li>
          </ul>
          <p className="rail-foot small muted">
            Local target · {report?.windows.product ?? "Windows"}
            <br />
            Telemetry off
          </p>
        </nav>
      )}
      {exporting && (
        <ExportRecording session={exporting} profile={profile} onClose={() => setExporting(null)} />
      )}
      <main className="main">
        {busy && (
          <p className="busy" role="status">
            {busy}
          </p>
        )}
        {error && space !== "terminal" && (
          <p role="alert" className="banner error">
            {error}
          </p>
        )}
        {waiting && !fullTerminal && (
          <div className="banner waiting" role="status">
            <p>
              <strong>{waiting.title}</strong> is waiting to continue. Phase{" "}
              <code>{waiting.after_phase}</code> finished before a {boundaryName(waiting.boundary)}.
            </p>
            <button
              className="primary"
              disabled={busy !== null}
              onClick={() =>
                void ipc
                  .openWaiting()
                  .then((o) => {
                    setWs(o.view);
                    setSealed(o.sealed);
                    setRunDone(null);
                    setWaiting(null);
                    setSpace("plan");
                  })
                  .catch((e) => setError(String(e)))
              }
            >
              Continue it
            </button>
          </div>
        )}
        {space === "task" && !fullTerminal && (
          <NewTask
            agents={agents}
            report={report}
            busy={busy !== null}
            onPlan={(task, kind, context) =>
              act("The agent is investigating…", () => ipc.propose(task, kind, context))
            }
            onOpen={(text) => act("Reading the plan…", () => ipc.openPlan(text))}
          />
        )}
        {space === "history" && !fullTerminal && (
          <HistoryView
            busy={busy !== null}
            onPromoted={() => setSpace("techniques")}
            onExport={setExporting}
          />
        )}
        {space === "techniques" && !fullTerminal && (
          <TechniquesView
            busy={busy !== null}
            onDraft={(draft) => {
              setWs(draft.view);
              setSealed(null);
              setSpace("plan");
            }}
          />
        )}
        {space === "plan" && ws && !fullTerminal && (
          <PlanWorkspace
            view={ws}
            agents={agents}
            busy={busy}
            sealed={sealed}
            canArm={canArm}
            act={act}
            onApprove={startApproval}
            onArm={() => void armPlan()}
            mode={mode}
            modes={MODES}
            onMode={(m) => setMode(m as ExecutionMode)}
            record={record}
            onRecord={setRecord}
          />
        )}
        <div
          className="app"
          data-armed={fullTerminal}
          hidden={space !== "terminal" && !fullTerminal}
        >
          {!fullTerminal && (
            <header className="topbar">
              <label>
                Shell{" "}
                <select
                  value={shell ?? ""}
                  onChange={(e) => restart(() => setShell(e.target.value as ShellKind))}
                >
                  {(report?.shells ?? []).map((s) => (
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
              <label title={PRESENTATIONS.find((p) => p.value === presentation)?.hint}>
                When KeyJutsu needs you{" "}
                <select
                  value={presentation}
                  onChange={(e) => setPresentation(e.target.value as Presentation)}
                >
                  {PRESENTATIONS.map((p) => (
                    <option key={p.value} value={p.value}>
                      {p.label}
                    </option>
                  ))}
                </select>
              </label>
              <button onClick={() => restart(() => setGeneration((g) => g + 1))}>
                New terminal
              </button>
            </header>
          )}

          <div className="body">
            {!fullTerminal && runDone && (
              <aside className="side" aria-label="Run">
                <RunPanel
                  done={runDone}
                  busy={busy !== null}
                  agents={agents ?? []}
                  onFix={async (fixWith, guidance) => {
                    setBusy("The agent is working out what went wrong…");
                    try {
                      setWs(await ipc.fixFailure(fixWith, guidance));
                      setRunDone(null);
                      setSpace("plan");
                    } finally {
                      setBusy(null);
                    }
                  }}
                  task={ws?.plan.title ?? ws?.plan.task_id ?? ""}
                  recorded={recorded}
                  onExport={setExporting}
                  onPromoted={() => {
                    setRunDone(null);
                    setSpace("techniques");
                  }}
                  onReview={() => ipc.recoveryPlan()}
                  onRecover={async () => {
                    if (sessionId === null) throw new Error("no terminal");
                    setBusy("Recovering…");
                    try {
                      return await ipc.recover(sessionId);
                    } finally {
                      setBusy(null);
                    }
                  }}
                  onBack={() => {
                    setRunDone(null);
                    setSpace("plan");
                  }}
                />
              </aside>
            )}
            {!fullTerminal && !runDone && (
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
                          These run exactly as if you typed them. They are not validated or
                          approved: for that, get a plan from New task.
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
                        : "Arming hands the keyboard to the performance. Ctrl+Alt+Shift+K disarms; Ctrl+Shift+K opens the controls; Ctrl+C stops a command that is running."}
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
                  <ReadinessPanel report={report} fold />
                </section>
                <section>
                  <h2>Diagnostics</h2>
                  <DiagnosticsPanel />
                </section>
                <section>
                  <h2>Updates</h2>
                  <UpdatePanel />
                </section>
              </aside>
            )}

            <section
              className="term-area"
              data-cue={
                waitingOnOperator && (r.edgeCue || (r.stagedError && staged === false))
                  ? "waiting"
                  : undefined
              }
            >
              <TerminalView
                ref={term}
                profile={profile}
                armed={fullTerminal}
                onData={onData}
                onKey={onKey}
                onResize={onResize}
                onTitle={(t) => (titleRef.current = t)}
              />
              {credential && r.credentialBanner && (
                <p className="term-banner" role="status">
                  Credential required: {credential}. Stop typing, then press Enter.
                </p>
              )}
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
            {waitingOnOperator && !r.coverTerminal
              ? "KeyJutsu is waiting for you. Press Ctrl+Shift+K."
              : armed && snapshot
                ? `Armed. Step ${snapshot.step_index + 1} of ${snapshot.step_count}.`
                : ""}
          </div>
        </div>
      </main>
      {dialog}
      {resumeNotice && showQuestion && (
        <ResumeDialog
          notice={resumeNotice}
          discreet={!r.coverTerminal}
          onConfirm={(typed) => {
            unstage();
            setResumeNotice(null);
            setRevealed(false);
            void ipc.confirm(typed);
            term.current?.focus();
          }}
          onCancel={() => {
            unstage();
            setResumeNotice(null);
            setRevealed(false);
            void ipc.confirm(null);
          }}
        />
      )}
    </div>
  );
}
