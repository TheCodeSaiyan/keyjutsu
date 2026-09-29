import { useEffect, useState } from "react";
import type { technique, TechniqueDraft } from "@keyjutsu/types";
import { agentLabel, when } from "../history";
import { ipc } from "../ipc";
import { lastChanged, latestFirst, whereItWorked } from "../technique";

interface Props {
  busy: boolean;
  /** The draft opens in the Plan screen, to validate and approve. */
  onDraft(draft: TechniqueDraft): void;
}

/**
 * Techniques: plans that worked, to use again, the latest first. Choosing
 * one shows what it does, where it came from and where it has worked; using
 * it fills in its parameters and makes a draft, and how this machine differs
 * from where it worked decides which steps have to be validated again.
 */
export function TechniquesView({ busy, onDraft }: Props) {
  const [list, setList] = useState<technique.Technique[] | null>(null);
  const [selected, setSelected] = useState<technique.Technique | null>(null);
  const [values, setValues] = useState<Record<string, string>>({});
  const [error, setError] = useState<string | null>(null);

  const choose = (t: technique.Technique) => {
    setSelected(t);
    setValues(Object.fromEntries(t.parameters.map((p) => [p.name, p.default ?? ""])));
  };

  useEffect(() => {
    ipc
      .techniques()
      .then((all) => {
        const sorted = latestFirst(all);
        setList(sorted);
        // The latest is usually the one wanted; nothing can be chosen before
        // the list arrives.
        if (sorted[0]) choose(sorted[0]);
      })
      .catch((e) => setError(String(e)));
  }, []);

  return (
    <section className="page" aria-label="Techniques">
      <p className="eyebrow">Techniques</p>
      <h1>Plans that worked, to use again</h1>
      <p className="muted">
        Every Technique is kept here, encrypted so only your Windows account on this machine can
        read it.
      </p>
      {error && (
        <p role="alert" className="error">
          {error}
        </p>
      )}
      {list && list.length === 0 && (
        <p className="muted">None yet. A run that completed can be made into one from History.</p>
      )}
      {list && list.length > 0 && (
        <div className="history-layout">
          <ul className="history-list" aria-label="Techniques, latest first">
            {list.map((t) => (
              <li key={t.id}>
                <button
                  className="run-row"
                  aria-current={selected?.id === t.id ? "true" : undefined}
                  onClick={() => choose(t)}
                >
                  <span className="run-title">{t.name}</span>
                  <span className="run-meta">
                    <span>{when(lastChanged(t))}</span>
                    <span>{whereItWorked(t)}</span>
                    {t.revision > 1 && <span>revision {t.revision}</span>}
                    {t.provenance.imported && <span className="chip">Imported</span>}
                  </span>
                </button>
              </li>
            ))}
          </ul>

          <article className="card run-detail" aria-label="The chosen Technique">
            {!selected && <p className="muted">Choose a Technique to see what it does.</p>}
            {selected && (
              <>
                <h2 className="run-heading">{selected.name}</h2>
                <p className="run-meta">
                  <span className="badge">revision {selected.revision}</span>
                  <span>saved {when(selected.provenance.created_at)}</span>
                  {selected.revision > 1 && selected.provenance.last_validated_at && (
                    <span>revalidated {when(selected.provenance.last_validated_at)}</span>
                  )}
                  <span>planned by {agentLabel(selected.provenance.agent)}</span>
                  <span>{whereItWorked(selected)}</span>
                </p>
                {selected.provenance.imported && (
                  <p className="run-stopped review">
                    It came from another machine, so every step is validated here before it can be
                    approved.
                  </p>
                )}
                {selected.description && selected.description !== selected.name && (
                  <p>{selected.description}</p>
                )}

                <p className="eyebrow muted">Steps</p>
                <ol className="technique-steps">
                  {selected.template.steps.map((st) => (
                    <li key={st.id}>{st.title}</li>
                  ))}
                </ol>

                {selected.parameters.length > 0 && (
                  <>
                    <p className="eyebrow muted">Parameters</p>
                    {selected.parameters.map((p) => (
                      <label key={p.name} className="small stack">
                        <span>
                          <strong>{p.name}</strong>
                          {p.description ? ` · ${p.description}` : ""}
                        </span>
                        <input
                          value={values[p.name] ?? ""}
                          onChange={(e) => setValues({ ...values, [p.name]: e.target.value })}
                        />
                      </label>
                    ))}
                  </>
                )}
                <p className="small muted">
                  This makes a draft plan. It is validated and approved here like any other: a
                  Technique never runs because it worked before.
                </p>
                <div className="row">
                  <button
                    className="primary"
                    disabled={busy}
                    onClick={async () => {
                      setError(null);
                      try {
                        onDraft(await ipc.useTechnique(selected.id, values));
                      } catch (e) {
                        setError(String(e));
                      }
                    }}
                  >
                    Make a draft plan
                  </button>
                </div>
                <p className="small muted">
                  Technique <code>{selected.id}</code>
                </p>
              </>
            )}
          </article>
        </div>
      )}
    </section>
  );
}
