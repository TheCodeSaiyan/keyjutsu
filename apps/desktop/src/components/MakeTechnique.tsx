import { useState } from "react";
import { ipc } from "../ipc";
import { parameterPairs } from "../technique";

interface Props {
  /** The history record of a run that completed. */
  session: string;
  /** The name it starts with: the run's task. */
  task: string;
  busy: boolean;
  /** Called after the Technique is saved, to show it. */
  onPromoted(): void;
}

/**
 * Turn a run that completed into a Technique: a plan to use again, with the
 * values that change from one use to the next made into parameters.
 */
export function MakeTechnique({ session, task, busy, onPromoted }: Props) {
  const [name, setName] = useState(task);
  const [params, setParams] = useState("");
  const [error, setError] = useState<string | null>(null);

  return (
    <div className="stack">
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
        Each value, wherever it appears in the plan, becomes a parameter you fill in when you use
        the Technique. A Technique never runs because it worked before: using one makes a draft that
        is validated and approved here.
      </p>
      <button
        className="primary"
        disabled={busy || !name.trim()}
        onClick={async () => {
          setError(null);
          try {
            await ipc.promote(session, name.trim(), "", parameterPairs(params));
            onPromoted();
          } catch (e) {
            setError(String(e));
          }
        }}
      >
        Make a Technique
      </button>
      {error && (
        <p role="alert" className="error">
          {error}
        </p>
      )}
    </div>
  );
}
