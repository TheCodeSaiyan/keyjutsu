import { useEffect, useRef, useState } from "react";
import type { history } from "@keyjutsu/types";
import { ipc } from "../ipc";
import {
  agentLabel,
  duration,
  newestFirst,
  outcomeBadge,
  stepTitles,
  stoppedBecause,
  when,
} from "../history";
import { MakeTechnique } from "./MakeTechnique";

interface Props {
  busy: boolean;
  /** Called after a Technique is saved, to show it. */
  onPromoted(): void;
  /** Export a recorded run. */
  onExport(session: string): void;
}

/**
 * Past runs, from the encrypted history on this machine, newest first: what
 * each did, step by step, when, for how long, and why it stopped if it did.
 * A recorded run can be exported; a run that completed can become a
 * Technique.
 */
export function HistoryView({ busy, onPromoted, onExport }: Props) {
  const [runs, setRuns] = useState<history.SessionSummary[] | null>(null);
  const [selected, setSelected] = useState<history.SessionRecord | null>(null);
  const [promoting, setPromoting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // The run the operator chose: a later answer for another run, such as the
  // newest opened on arrival, must not replace it.
  const wanted = useRef<string | null>(null);

  const open = async (id: string) => {
    wanted.current = id;
    setError(null);
    setPromoting(false);
    try {
      const record = await ipc.historyShow(id);
      if (wanted.current === id) setSelected(record);
    } catch (e) {
      setError(String(e));
    }
  };

  useEffect(() => {
    ipc
      .history()
      .then((list) => {
        const sorted = newestFirst(list);
        setRuns(sorted);
        // The latest run is usually the one wanted, unless one was chosen.
        if (sorted[0] && wanted.current === null) void open(sorted[0].id);
      })
      .catch((e) => setError(String(e)));
  }, []);

  const summary = runs?.find((r) => r.id === selected?.id);
  const titles = selected ? stepTitles(selected.snapshot) : new Map<string, string>();
  const ran = selected?.checkpoint?.runs ?? [];
  const notRun = [...titles.keys()].filter((id) => !ran.some((r) => r.step === id));
  const stopped = selected ? stoppedBecause(selected.outcome, titles) : null;

  return (
    <section className="page history" aria-label="History">
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
      {runs && runs.length === 0 && <p className="muted">Nothing has run yet.</p>}
      {runs && runs.length > 0 && (
        <div className="history-layout">
          <ul className="history-list" aria-label="Runs, newest first">
            {runs.map((r) => {
              const badge = outcomeBadge(r.outcome);
              return (
                <li key={r.id}>
                  <button
                    className="run-row"
                    aria-current={selected?.id === r.id ? "true" : undefined}
                    onClick={() => void open(r.id)}
                  >
                    <span className="run-title">{r.task}</span>
                    <span className="run-meta">
                      <span className={`badge ${badge.tone}`}>{badge.label}</span>
                      <span>{when(r.finished_at)}</span>
                      {r.recorded && <span className="chip">Recorded</span>}
                    </span>
                  </button>
                </li>
              );
            })}
          </ul>

          <article className="card run-detail" aria-label="The chosen run">
            {!selected && <p className="muted">Choose a run to see what it did.</p>}
            {selected && (
              <>
                <h2 className="run-heading">{selected.task}</h2>
                <p className="run-meta">
                  {summary && (
                    <span className={`badge ${outcomeBadge(summary.outcome).tone}`}>
                      {outcomeBadge(summary.outcome).label}
                    </span>
                  )}
                  <span>{when(selected.started_at)}</span>
                  <span>took {duration(selected.started_at, selected.finished_at)}</span>
                  <span>planned by {agentLabel(selected.agent)}</span>
                </p>
                {stopped && (
                  <p className={`run-stopped ${summary ? outcomeBadge(summary.outcome).tone : ""}`}>
                    {stopped}
                  </p>
                )}

                <p className="eyebrow muted">Steps</p>
                <ol className="run-steps">
                  {ran.map((r) => (
                    <li key={r.step} data-result={r.succeeded ? "passed" : "failed"}>
                      <span aria-hidden="true">{r.succeeded ? "✓" : "✗"}</span>
                      <span>{titles.get(r.step) ?? r.step}</span>
                      <span className="muted small">{duration(r.started_at, r.finished_at)}</span>
                      <span className="visually-hidden">
                        {r.succeeded ? " (succeeded)" : " (failed)"}
                      </span>
                    </li>
                  ))}
                  {notRun.map((id) => (
                    <li key={id} data-result="not_run">
                      <span aria-hidden="true">–</span>
                      <span>{titles.get(id) ?? id}</span>
                      <span className="muted small">not run</span>
                    </li>
                  ))}
                </ol>

                <div className="row">
                  {summary?.recorded && (
                    <button onClick={() => onExport(selected.id)}>Export the recording…</button>
                  )}
                  {selected.outcome.kind === "complete" && !promoting && (
                    <button onClick={() => setPromoting(true)}>Make it a Technique…</button>
                  )}
                </div>
                {selected.outcome.kind === "complete" && promoting && (
                  <MakeTechnique
                    key={selected.id}
                    session={selected.id}
                    task={selected.task}
                    busy={busy}
                    onPromoted={onPromoted}
                  />
                )}
                <p className="small muted">
                  Session <code>{selected.id}</code>
                </p>
              </>
            )}
          </article>
        </div>
      )}
    </section>
  );
}
