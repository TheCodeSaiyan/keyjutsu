import { useEffect, useState } from "react";
import { ipc } from "../ipc";

interface Props {
  session: string;
}

/**
 * A run's change record, to attach to a ticket: built from the encrypted
 * history when asked for, shown before anything is written, then copied or
 * saved as Markdown.
 */
export function ChangeRecord({ session }: Props) {
  const [text, setText] = useState<string | null>(null);
  const [saved, setSaved] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let current = true;
    ipc
      .changeRecord(session)
      .then((t) => current && setText(t))
      .catch((e) => current && setError(String(e)));
    return () => {
      current = false;
    };
  }, [session]);

  const save = async () => {
    setError(null);
    try {
      setSaved(await ipc.saveChangeRecord(session));
    } catch (e) {
      setError(String(e));
    }
  };

  const copy = async () => {
    if (text === null) return;
    setError(null);
    try {
      await navigator.clipboard.writeText(text);
      setCopied(true);
    } catch (e) {
      setError(String(e));
    }
  };

  return (
    <div className="stack">
      <p className="small muted">
        What was approved under which work profile, by whom, what ran and how it ended. Names are
        kept and secrets are redacted. Nothing is written until you save it.
      </p>
      {text === null && !error && (
        <p role="status" className="small muted">
          Building the change record…
        </p>
      )}
      {text !== null && (
        <>
          <div className="row">
            <button onClick={() => void copy()}>{copied ? "Copied" : "Copy"}</button>
            <button onClick={() => void save()}>Save as Markdown</button>
          </div>
          {saved && (
            <p role="status" className="small">
              Saved to <code>{saved}</code>.
            </p>
          )}
          <pre className="code bundle" tabIndex={0} aria-label="The change record">
            {text}
          </pre>
        </>
      )}
      {error && (
        <p role="alert" className="error">
          {error}
        </p>
      )}
    </div>
  );
}
