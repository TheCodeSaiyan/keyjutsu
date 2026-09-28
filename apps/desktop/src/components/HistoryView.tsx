import { useEffect, useState } from "react";
import type { history } from "@keyjutsu/types";
import { ipc } from "../ipc";
import { MakeTechnique } from "./MakeTechnique";

interface Props {
  busy: boolean;
  /** Called after a Technique is saved, to show it. */
  onPromoted(): void;
  /** Export a recorded run. */
  onExport(session: string): void;
}

/**
 * Past runs, from the encrypted history on this machine. A run that
 * completed can become a Technique: a plan to use again, with the values
 * that change from one use to the next made into parameters.
 */
export function HistoryView({ busy, onPromoted, onExport }: Props) {
  const [sessions, setSessions] = useState<history.SessionSummary[] | null>(null);
  const [selected, setSelected] = useState<history.SessionRecord | null>(null);
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
    } catch (e) {
      setError(String(e));
    }
  };

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
                  {s.recorded ? " · recorded" : ""}
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
            {sessions?.find((s) => s.id === selected.id)?.recorded && (
              <button onClick={() => onExport(selected.id)}>Export the recording…</button>
            )}
            <ol className="small">
              {(selected.checkpoint?.runs ?? []).map((r) => (
                <li key={r.step}>
                  {r.succeeded ? "ok" : "FAILED"} · {r.step}
                </li>
              ))}
            </ol>
            {selected.outcome.kind === "complete" ? (
              <MakeTechnique
                key={selected.id}
                session={selected.id}
                task={selected.task}
                busy={busy}
                onPromoted={onPromoted}
              />
            ) : (
              <p className="small muted">Only a run that completed can become a Technique.</p>
            )}
          </div>
        )}
      </div>
    </section>
  );
}
