import { useEffect, useState } from "react";
import type { history } from "@keyjutsu/types";
import { ipc } from "../ipc";

interface Props {
  busy: boolean;
  /** Called after a Technique is saved, to show it. */
  onPromoted(): void;
}

/**
 * Past runs, from the encrypted history on this machine. A run that
 * completed can become a Technique: a plan to use again, with the values
 * that change from one use to the next made into parameters.
 */
export function HistoryView({ busy, onPromoted }: Props) {
  const [sessions, setSessions] = useState<history.SessionSummary[] | null>(null);
  const [selected, setSelected] = useState<history.SessionRecord | null>(null);
  const [name, setName] = useState("");
  const [params, setParams] = useState("");
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    ipc
      .history()
      .then(setSessions)
      .catch((e) => setError(String(e)));
  }, []);

  const open = async (id: string) => {
    setError(null);
    try {
      const record = await ipc.historyShow(id);
      setSelected(record);
      setName(record.task);
      setParams("");
    } catch (e) {
      setError(String(e));
    }
  };

  // One parameter per line, "name = value": the value as it appears in the
  // run's plan, which the Technique asks for next time.
  const pairs = (): [string, string][] =>
    params
      .split("\n")
      .map((l) => l.split("="))
      .filter((p) => p.length === 2 && p[0].trim() && p[1].trim())
      .map(([n, v]) => [n.trim(), v.trim()]);

  return (
    <section className="page" aria-label="History">
      <p className="eyebrow">History</p>
      <h1>What ran on this machine</h1>
      <p className="muted">
        Every run is kept here, encrypted so only your Windows account on this machine can read it.
      </p>
      {error && (
        <p role="alert" className="error">
          {error}
        </p>
      )}
      {sessions && sessions.length === 0 && <p className="muted">Nothing has run yet.</p>}
      <div className="columns">
        <ul className="plain list">
          {(sessions ?? []).map((s) => (
            <li key={s.id}>
              <button
                aria-current={selected?.id === s.id ? "true" : undefined}
                onClick={() => void open(s.id)}
              >
                <strong>{s.task}</strong>
                <span className="small muted">
                  {s.outcome} · {s.finished_at.replace("T", " ").replace("Z", "")}
                </span>
              </button>
            </li>
          ))}
        </ul>
        {selected && (
          <div className="card stack">
            <h2>{selected.task}</h2>
            <p className="small muted">
              Session <code>{selected.id}</code> · {selected.outcome.kind}
            </p>
            <ol className="small">
              {(selected.checkpoint?.runs ?? []).map((r) => (
                <li key={r.step}>
                  {r.succeeded ? "ok" : "FAILED"} · {r.step}
                </li>
              ))}
            </ol>
            {selected.outcome.kind === "complete" ? (
              <>
                <h2>Make it a Technique</h2>
                <label className="small">
                  Name
                  <input value={name} onChange={(e) => setName(e.target.value)} />
                </label>
                <label className="small">
                  Parameters, one per line, as <code>name = value</code>
                  <textarea
                    value={params}
                    onChange={(e) => setParams(e.target.value)}
                    placeholder="service_name = Winmgmt"
                  />
                </label>
                <p className="small muted">
                  Each value, wherever it appears in the plan, becomes a parameter you fill in when
                  you use the Technique. A Technique never runs because it worked before: using one
                  makes a draft that is validated and approved here.
                </p>
                <button
                  className="primary"
                  disabled={busy || !name.trim()}
                  onClick={async () => {
                    setError(null);
                    try {
                      await ipc.promote(selected.id, name.trim(), "", pairs());
                      onPromoted();
                    } catch (e) {
                      setError(String(e));
                    }
                  }}
                >
                  Make a Technique
                </button>
              </>
            ) : (
              <p className="small muted">Only a run that completed can become a Technique.</p>
            )}
          </div>
        )}
      </div>
    </section>
  );
}
