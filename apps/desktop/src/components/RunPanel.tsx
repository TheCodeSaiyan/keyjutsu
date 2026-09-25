import { useState } from "react";
import type { recovery, RunMessage } from "@keyjutsu/types";
import { outcomeLine } from "../plan";

type Done = Extract<RunMessage, { kind: "done" }>;

interface Props {
  done: Done;
  busy: boolean;
  onReview(): Promise<recovery.RecoveryItem[]>;
  onRecover(): Promise<recovery.RecoveryResult[]>;
  onBack(): void;
}

/**
 * After a run: what happened, where the record is, and, after a failure,
 * the four choices of §29. Nothing is rolled back unless the operator
 * reviews the recovery plan and then confirms it.
 */
export function RunPanel({ done, busy, onReview, onRecover, onBack }: Props) {
  const [items, setItems] = useState<recovery.RecoveryItem[] | null>(null);
  const [results, setResults] = useState<recovery.RecoveryResult[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const complete = done.outcome.kind === "complete";

  return (
    <section className="run-panel" aria-label="Run">
      <h2>{complete ? "Complete" : "Stopped"}</h2>
      <p role="status">{outcomeLine(done.outcome)}</p>
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
      {!complete && !items && (
        <>
          <p className="small">Nothing has been rolled back. Your choices:</p>
          <ul className="small plain">
            <li>Diagnose first: the terminal is as the step left it.</li>
            <li>Review the recovery plan, then roll back if it is right.</li>
            <li>Stop here without rolling back.</li>
          </ul>
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
