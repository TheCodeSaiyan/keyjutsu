import { useEffect, useRef } from "react";
import type { PerformanceSnapshot } from "@keyjutsu/types";

interface Props {
  snapshot: PerformanceSnapshot | null;
  paused: boolean;
  onResume(): void;
  onDisarm(): void;
}

/**
 * The operator's private controls during a performance, opened with
 * Ctrl+Shift+K. Small and in a corner: it is for the operator, not the room.
 */
export function Overlay({ snapshot, paused, onResume, onDisarm }: Props) {
  const first = useRef<HTMLButtonElement>(null);
  useEffect(() => first.current?.focus(), []);
  return (
    <div
      className="overlay"
      role="dialog"
      aria-modal="true"
      aria-label="KeyJutsu operator controls"
    >
      {snapshot && (
        <p>
          Step {snapshot.step_index + 1} of {snapshot.step_count} ·{" "}
          {snapshot.state.replaceAll("_", " ").toLowerCase()}
          {snapshot.total_chars > 0 && ` · ${snapshot.typed_chars}/${snapshot.total_chars}`}
        </p>
      )}
      <p className="muted">{paused ? "Paused. Staged input will not advance." : "Running."}</p>
      <div className="row">
        <button ref={first} onClick={onResume}>
          Resume
        </button>
        <button className="danger" onClick={onDisarm}>
          Disarm
        </button>
      </div>
      <p className="muted small">Ctrl+Alt+Shift+K disarms at any time.</p>
    </div>
  );
}
