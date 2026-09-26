import { useState } from "react";
import type { agent, recovery, RunMessage } from "@keyjutsu/types";
import { boundaryStep, outcomeLine } from "../plan";

type Done = Extract<RunMessage, { kind: "done" }>;

interface Props {
  done: Done;
  busy: boolean;
  /** Installed agents, for asking one to fix a failed step. */
  agents: agent.AgentInfo[];
  onReview(): Promise<recovery.RecoveryItem[]>;
  onRecover(): Promise<recovery.RecoveryResult[]>;
  /** Ask the agent to fix the failed step; the revised plan opens for review. */
  onFix(agent: agent.AgentKind, guidance: string): Promise<void>;
  onBack(): void;
}

/**
 * After a run: what happened, where the record is, and, after a failure,
 * the four choices it has. Nothing is rolled back unless the operator
 * reviews the recovery plan and then confirms it.
 */
export function RunPanel({ done, busy, agents, onReview, onRecover, onFix, onBack }: Props) {
  const [items, setItems] = useState<recovery.RecoveryItem[] | null>(null);
  const installed = agents.filter((a) => a.path !== null);
  const [fixWith, setFixWith] = useState<agent.AgentKind | null>(installed[0]?.kind ?? null);
  const [guidance, setGuidance] = useState("");
  const failed = done.outcome.kind === "failed" ? done.outcome : null;
  const [results, setResults] = useState<recovery.RecoveryResult[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const complete = done.outcome.kind === "complete";
  const boundary = done.outcome.kind === "boundary" ? done.outcome : null;

  return (
    <section className="run-panel" aria-label="Run">
      <h2>{complete ? "Complete" : boundary ? "Waiting" : "Stopped"}</h2>
      <p role="status">{outcomeLine(done.outcome)}</p>
      {boundary && (
        <p className="small">
          {boundaryStep(boundary.boundary)} Before anything more runs, KeyJutsu checks that it
          happened, compares this machine with the one the plan was approved on, checks again what
          the earlier phases achieved, and asks you.
        </p>
      )}
      <p className="muted small">
        Checkpoint: <code>{done.checkpoint}</code>
      </p>
      {done.git.map((r) => (
        <div key={r.root} className="git-report">
          <h2>Git · {r.branch ?? r.root}</h2>
          {r.keyjutsu.length === 0 ? (
            <p className="small muted">KeyJutsu changed nothing in {r.root}.</p>
          ) : (
            <ul className="small plain">
              {r.keyjutsu.map((k) => (
                <li key={k.path}>
                  <details>
                    <summary>
                      Changed by KeyJutsu: <code>{k.path}</code>
                      {k.was_already_changed
                        ? " (you had changed it too; only KeyJutsu's part is shown)"
                        : ""}
                    </summary>
                    <pre className="code">{k.diff}</pre>
                  </details>
                </li>
              ))}
            </ul>
          )}
          {r.untouched.length > 0 && (
            <p className="small muted">Your own changes, untouched: {r.untouched.join(", ")}</p>
          )}
        </div>
      ))}
      {failed && failed.output.trim() && (
        <details open>
          <summary className="small">What the step printed</summary>
          {/* The same text the agent is shown, before redaction. */}
          <pre className="code">{failed.output.trim()}</pre>
        </details>
      )}
      {!complete && !boundary && !items && (
        <>
          <p className="small">Nothing has been rolled back. Your choices:</p>
          <ul className="small plain">
            <li>Diagnose first: the terminal is as the step left it.</li>
            <li>Ask an agent to fix the step from what it printed, then approve and carry on.</li>
            <li>Review the recovery plan, then roll back if it is right.</li>
            <li>Stop here without rolling back.</li>
          </ul>
          {failed && (
            <div className="stack">
              <label className="small">
                Guidance for the fix
                <textarea
                  value={guidance}
                  onChange={(e) => setGuidance(e.target.value)}
                  placeholder="Start the service it depends on first."
                />
              </label>
              <label className="small">
                Agent
                <select
                  aria-label="Agent to fix the step"
                  value={fixWith ?? ""}
                  onChange={(e) => setFixWith(e.target.value as agent.AgentKind)}
                  disabled={installed.length === 0}
                >
                  {installed.length === 0 && <option value="">No agent installed</option>}
                  {installed.map((a) => (
                    <option key={a.kind} value={a.kind}>
                      {a.name}
                    </option>
                  ))}
                </select>
              </label>
              <p className="small muted">
                The agent is shown this step, your guidance and what the step printed, with anything
                that looks like a secret taken out. The fix comes back as a draft: nothing runs
                until it is validated and you approve it, and then the run carries on from this
                step.
              </p>
              <button
                disabled={busy || !fixWith}
                onClick={async () => {
                  setError(null);
                  try {
                    await onFix(fixWith!, guidance.trim());
                  } catch (e) {
                    setError(String(e));
                  }
                }}
              >
                Ask the agent to fix it
              </button>
            </div>
          )}
          <div className="stack">
            <button
              disabled={busy}
              onClick={async () => {
                setError(null);
                try {
                  setItems(await onReview());
                } catch (e) {
                  setError(String(e));
                }
              }}
            >
              Review recovery plan
            </button>
          </div>
        </>
      )}
      {items && !results && (
        <>
          <h2>Recovery plan, latest first</h2>
          <ol className="small plain">
            {items.map((i) => (
              <li key={i.step}>
                <strong>{i.step}</strong>:{" "}
                {i.action === "restore"
                  ? i.what.join("; ")
                  : i.action === "commands"
                    ? `run ${i.commands.join("; ")}`
                    : `cannot be recovered (${i.reason})`}
              </li>
            ))}
          </ol>
          <div className="row">
            <button
              className="secondary-strong"
              disabled={busy}
              onClick={async () => {
                setError(null);
                try {
                  setResults(await onRecover());
                } catch (e) {
                  setError(String(e));
                }
              }}
            >
              Roll back now
            </button>
            <button onClick={() => setItems(null)}>Not now</button>
          </div>
        </>
      )}
      {results && (
        <>
          <h2>Recovery</h2>
          <ul className="small plain">
            {results.map((r) => (
              <li key={r.step}>
                <strong>{r.recovered ? "Recovered" : "Not recovered"}</strong> {r.step}
                <ul className="plain">
                  {r.checks.map((c, n) => (
                    <li key={n}>
                      {c.passed === true ? "✓" : c.passed === false ? "✗" : "–"} {c.check}{" "}
                      {c.detail}
                    </li>
                  ))}
                </ul>
              </li>
            ))}
          </ul>
        </>
      )}
      {error && (
        <p role="alert" className="error">
          {error}
        </p>
      )}
      <button onClick={onBack}>Back to the plan</button>
    </section>
  );
}
