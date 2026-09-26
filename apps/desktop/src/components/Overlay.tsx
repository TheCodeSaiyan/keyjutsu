import { useEffect, useRef } from "react";
import type { PerformanceSnapshot } from "@keyjutsu/types";
import { modeLabel, ordinal, stateLabel } from "../labels";

interface Props {
  snapshot: PerformanceSnapshot | null;
  paused: boolean;
  onResume(): void;
  onDisarm(): void;
}

/**
 * The operator's private controls during a performance, opened with
 * Ctrl+Shift+K. Small and in a corner: it is for the operator, not the room.
 * Shows what the design kit asks for: current step, state, next step,
 * mode and progress, with Resume and Disarm.
 */
export function Overlay({ snapshot, paused, onResume, onDisarm }: Props) {
  const first = useRef<HTMLButtonElement>(null);
  useEffect(() => first.current?.focus(), []);
  const done = snapshot?.outcomes.length ?? 0;
  return (
    <div
      className="overlay"
      role="dialog"
      aria-modal="true"
      aria-label="KeyJutsu operator controls"
    >
      <div className="overlay-head">
        <span>Operator controls</span>
        {snapshot && <span className="chip">{modeLabel(snapshot.step_mode)}</span>}
      </div>
      {snapshot && (
        <>
          <dl>
            <dt>Current step</dt>
            <dd>
              {ordinal(snapshot.step_index)} · {snapshot.step_title}
            </dd>
            <dt>State</dt>
            <dd className="state">{paused ? "Paused" : stateLabel(snapshot.state)}</dd>
            {snapshot.asks_operator && (
              <>
                <dt>Your turn</dt>
                <dd>
                  Staged typing is off. Stop typing, press Enter, then answer the shell&apos;s
                  prompt yourself.
                </dd>
              </>
            )}
            {snapshot.total_chars > 0 && !snapshot.asks_operator && (
              <>
                <dt>Typed</dt>
                <dd>
                  {snapshot.typed_chars} / {snapshot.total_chars}
                </dd>
              </>
            )}
            <dt>Next</dt>
            <dd>{snapshot.next_step_title ?? "Nothing: this is the last step"}</dd>
            <dt>Progress</dt>
            <dd>
              {done} / {snapshot.step_count}
            </dd>
          </dl>
          <progress
            value={done}
            max={snapshot.step_count}
            aria-label={`${done} of ${snapshot.step_count} steps finished`}
          />
        </>
      )}
      <div className="row">
        <button ref={first} onClick={onResume}>
          Resume
        </button>
        <button className="danger" onClick={onDisarm}>
          Disarm
        </button>
      </div>
      <p className="hint">Ctrl+Shift+K · controls &nbsp; Ctrl+Alt+Shift+K · disarm</p>
    </div>
  );
}
