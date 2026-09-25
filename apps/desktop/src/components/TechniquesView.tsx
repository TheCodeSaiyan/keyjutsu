import { useEffect, useState } from "react";
import type { technique, TechniqueDraft } from "@keyjutsu/types";
import { ipc } from "../ipc";

interface Props {
  busy: boolean;
  /** The draft opens in the Plan screen, to validate and approve. */
  onDraft(draft: TechniqueDraft): void;
}

/**
 * Techniques: plans that worked, to use again. Using one fills in its
 * parameters and makes a draft; how this machine differs from where it
 * worked decides which steps have to be validated again.
 */
export function TechniquesView({ busy, onDraft }: Props) {
  const [list, setList] = useState<technique.Technique[] | null>(null);
  const [selected, setSelected] = useState<technique.Technique | null>(null);
  const [values, setValues] = useState<Record<string, string>>({});
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    ipc
      .techniques()
      .then(setList)
      .catch((e) => setError(String(e)));
  }, []);

  const choose = (t: technique.Technique) => {
    setSelected(t);
    setValues(Object.fromEntries(t.parameters.map((p) => [p.name, p.default ?? ""])));
  };

  return (
    <section className="page" aria-label="Techniques">
      <p className="eyebrow">Techniques</p>
      <h1>Plans that worked, to use again</h1>
      {error && (
        <p role="alert" className="error">
          {error}
        </p>
      )}
      {list && list.length === 0 && (
        <p className="muted">None yet. A run that completed can be made into one from History.</p>
      )}
      <div className="columns">
        <ul className="plain list">
          {(list ?? []).map((t) => (
            <li key={t.id}>
              <button
                aria-current={selected?.id === t.id ? "true" : undefined}
                onClick={() => choose(t)}
              >
                <strong>{t.name}</strong>
                <span className="small muted">
                  revision {t.revision}
                  {t.provenance.imported ? " · imported" : ""} · worked on{" "}
                  {t.provenance.known_good.length} machine
                  {t.provenance.known_good.length === 1 ? "" : "s"}
                </span>
              </button>
            </li>
          ))}
        </ul>
        {selected && (
          <div className="card stack">
            <h2>{selected.name}</h2>
            {selected.parameters.length === 0 && (
              <p className="small muted">It has no parameters.</p>
            )}
            {selected.parameters.map((p) => (
              <label key={p.name} className="small">
                {p.name}
                <input
                  value={values[p.name] ?? ""}
                  onChange={(e) => setValues({ ...values, [p.name]: e.target.value })}
                />
              </label>
            ))}
            <p className="small muted">
              This makes a draft plan. It is validated and approved here like any other: a Technique
              never runs because it worked before.
            </p>
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
        )}
      </div>
    </section>
  );
}
