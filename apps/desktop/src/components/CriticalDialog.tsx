import { useEffect, useRef, useState } from "react";
import type { execute } from "@keyjutsu/types";

interface Props {
  confirmation: execute.CriticalConfirmation;
  /** "approve" at approval; "run" just before the step runs. */
  purpose: "approve" | "run";
  onConfirm(typed: string): void;
  onCancel(): void;
  /** A corner card with the terminal still showing, not a full cover. */
  discreet?: boolean;
}

/**
 * A critical action, shown in full before it is approved and again before it
 * runs (kit §13): what it acts on, what it does, whether it can be undone,
 * what validation found, and the phrase to type. The button only enables on
 * an exact match to save a round trip; Rust compares the phrase itself.
 */
export function CriticalDialog({ confirmation: c, purpose, onConfirm, onCancel, discreet }: Props) {
  const [typed, setTyped] = useState("");
  const input = useRef<HTMLInputElement>(null);
  useEffect(() => input.current?.focus(), []);
  const matches = typed.trim() === c.phrase;

  return (
    <div className={discreet ? "scrim discreet" : "scrim"}>
      <div
        className="critical-dialog"
        role="alertdialog"
        aria-modal="true"
        aria-labelledby="critical-title"
        aria-describedby="critical-lede"
        onKeyDown={(e) => {
          if (e.key === "Escape") onCancel();
        }}
      >
        <header>
          <span className="danger-mark" aria-hidden="true">
            !
          </span>
          <div>
            <p className="eyebrow critical-text">Critical action</p>
            <h1 id="critical-title">{c.title}</h1>
            <p id="critical-lede" className="muted">
              {purpose === "run"
                ? "This approved step is next. Nothing runs until you confirm it again."
                : "This step is validated, but its effects are serious. Approving the plan does not cover it on its own."}
            </p>
          </div>
        </header>
        <dl>
          {c.targets.length > 0 && (
            <>
              <dt>Target</dt>
              <dd>
                {c.targets.map((t) => (
                  <code key={t} className="target">
                    {t}
                  </code>
                ))}
              </dd>
            </>
          )}
          <dt>Runs</dt>
          <dd>
            <pre className="code">{c.commands.join("\n")}</pre>
          </dd>
          {c.impact.length > 0 && (
            <>
              <dt>Impact</dt>
              <dd>
                {/* The agent's sentence, then KeyJutsu's reasons, which are
                    fragments: joined with spaces they ran together. */}
                <ul className="plain">
                  {c.impact.map((i) => (
                    <li key={i}>{i}</li>
                  ))}
                </ul>
              </dd>
            </>
          )}
          <dt>Recovery</dt>
          <dd
            className={
              c.recovery.startsWith("NONE") || c.recovery.startsWith("none") ? "critical-text" : ""
            }
          >
            {c.recovery}
          </dd>
          {c.evidence.length > 0 && (
            <>
              <dt>Validation</dt>
              <dd>{c.evidence.join(" · ")}</dd>
            </>
          )}
        </dl>
        <form
          onSubmit={(e) => {
            e.preventDefault();
            if (matches) onConfirm(typed.trim());
          }}
        >
          <label htmlFor="critical-phrase" className="small">
            To {purpose === "run" ? "run" : "approve"} this step, type <strong>{c.phrase}</strong>
          </label>
          <input
            id="critical-phrase"
            ref={input}
            className="phrase"
            autoComplete="off"
            spellCheck={false}
            value={typed}
            onChange={(e) => setTyped(e.target.value)}
          />
          <div className="row end">
            <button type="button" onClick={onCancel}>
              {purpose === "run" ? "Stop here" : "Cancel"}
            </button>
            <button type="submit" className="destructive" disabled={!matches}>
              {purpose === "run" ? "Run critical step" : "Approve critical step"}
            </button>
          </div>
        </form>
      </div>
    </div>
  );
}
