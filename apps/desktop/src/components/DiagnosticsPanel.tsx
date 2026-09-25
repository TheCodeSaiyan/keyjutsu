import { useState } from "react";
import { ipc } from "../ipc";

/**
 * A diagnostic bundle for someone helping with a problem. It is shown in
 * full first, and only the bundle shown is saved; KeyJutsu sends nothing.
 */
export function DiagnosticsPanel() {
  const [text, setText] = useState<string | null>(null);
  const [saved, setSaved] = useState<string | null>(null);
  const [working, setWorking] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const act = async (f: () => Promise<void>) => {
    setError(null);
    setWorking(true);
    try {
      await f();
    } catch (e) {
      setError(String(e));
    } finally {
      setWorking(false);
    }
  };

  return (
    <div className="stack">
      <p className="small muted">
        Versions, checks and counts, for someone helping you. No tasks, commands, output or secrets,
        and nothing is sent.
      </p>
      <div className="row">
        <button
          disabled={working}
          onClick={() =>
            void act(async () => {
              setSaved(null);
              setText(await ipc.diagnosticsPreview());
            })
          }
        >
          Preview bundle
        </button>
        <button
          disabled={working || text === null}
          onClick={() => void act(async () => setSaved(await ipc.diagnosticsSave()))}
        >
          Save bundle
        </button>
      </div>
      {working && (
        <p role="status" className="small muted">
          Checking this machine…
        </p>
      )}
      {saved && (
        <p role="status" className="small">
          Saved to <code>{saved}</code>. Read it before you send it to anyone.
        </p>
      )}
      {error && (
        <p role="alert" className="error">
          {error}
        </p>
      )}
      {text !== null && (
        <pre className="code bundle" tabIndex={0} aria-label="The diagnostic bundle">
          {text}
        </pre>
      )}
    </div>
  );
}
