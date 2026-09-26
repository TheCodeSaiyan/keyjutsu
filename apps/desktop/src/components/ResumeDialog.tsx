import { useEffect, useRef, useState } from "react";
import type { execute } from "@keyjutsu/types";
import { boundaryName } from "../plan";

interface Props {
  notice: execute.BoundaryNotice;
  onConfirm(typed: string): void;
  onCancel(): void;
}

const WORD = "RESUME";

/**
 * Before a plan crosses a restart or other session boundary: what KeyJutsu
 * checked on this side of it, and the word to type to go on. The button
 * only enables on a match to save a round trip; Rust compares the word.
 */
export function ResumeDialog({ notice: n, onConfirm, onCancel }: Props) {
  const [typed, setTyped] = useState("");
  const input = useRef<HTMLInputElement>(null);
  useEffect(() => input.current?.focus(), []);
  const what = boundaryName(n.kind);

  return (
    <div className="scrim">
      <div
        className="critical-dialog"
        role="alertdialog"
        aria-modal="true"
        aria-labelledby="resume-title"
        aria-describedby="resume-lede"
        onKeyDown={(e) => {
          if (e.key === "Escape") onCancel();
        }}
      >
        <header>
          <div>
            <p className="eyebrow">Continue the plan</p>
            <h1 id="resume-title">After the {what}</h1>
            <p id="resume-lede" className="muted">
              Phase <code>{n.after_phase}</code> finished before the {what}.
              {n.next_phase ? (
                <>
                  {" "}
                  Next is phase <code>{n.next_phase}</code>.
                </>
              ) : null}{" "}
              Nothing more runs until you confirm.
            </p>
          </div>
        </header>
        <dl>
          <dt>The {what}</dt>
          <dd>
            {n.verified
              ? "KeyJutsu saw it happen."
              : "KeyJutsu has no way to tell whether it happened: that is your call."}
          </dd>
          <dt>This machine</dt>
          <dd>
            {n.drifts.length === 0 ? (
              "The same as when the plan was approved."
            ) : (
              <ul className="plain">
                {n.drifts.map((d) => (
                  <li key={d.what}>
                    {d.what}: {d.before ?? "absent"} → {d.after ?? "absent"} (no step still to run
                    depends on it)
                  </li>
                ))}
              </ul>
            )}
          </dd>
          <dt>What the earlier phases achieved</dt>
          <dd>
            {n.rechecked.length === 0 ? (
              "Nothing they did has a check that can be repeated."
            ) : (
              <ul className="plain">
                {n.rechecked.map((r) => (
                  <li key={r.check}>
                    {r.passed === true ? "holds" : "could not be decided"} · {r.check}
                    {r.detail ? `: ${r.detail}` : ""}
                  </li>
                ))}
              </ul>
            )}
          </dd>
        </dl>
        <form
          onSubmit={(e) => {
            e.preventDefault();
            if (typed.trim() === WORD) onConfirm(typed.trim());
          }}
        >
          <label htmlFor="resume-word" className="small">
            To continue, type <strong>{WORD}</strong>
          </label>
          <input
            id="resume-word"
            ref={input}
            className="phrase"
            autoComplete="off"
            spellCheck={false}
            value={typed}
            onChange={(e) => setTyped(e.target.value)}
          />
          <div className="row end">
            <button type="button" onClick={onCancel}>
              Not now
            </button>
            <button type="submit" className="primary" disabled={typed.trim() !== WORD}>
              Continue the plan
            </button>
          </div>
        </form>
      </div>
    </div>
  );
}
