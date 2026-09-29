import { useEffect, useState } from "react";
import { elapsed } from "../labels";

/**
 * What is in progress, and for how long. An agent's request can be stopped:
 * an agent can take minutes, and without a clock or a way out the window
 * looks stuck. The agent is stopped after ten minutes whatever happens.
 */
export function BusyLine({
  label,
  since,
  agent,
  queued,
  onStop,
}: {
  label: string;
  /** When it started, in milliseconds since the epoch. */
  since: number;
  /** An agent's request, which Stop ends. */
  agent: boolean;
  /** Replies waiting behind it. */
  queued: number;
  onStop(): void;
}) {
  const [now, setNow] = useState(since);
  const [stopping, setStopping] = useState(false);
  useEffect(() => {
    const t = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(t);
  }, []);
  const seconds = Math.max(0, Math.floor((now - since) / 1000));

  return (
    <div className="busy" role="status">
      <p>
        {label} <span className="muted">{elapsed(seconds)}</span>
        {queued > 0 && ` · ${queued} more ${queued === 1 ? "reply" : "replies"} queued`}
        {agent && seconds >= 60 && (
          <span className="small muted">
            {" "}
            Agents often take a few minutes; KeyJutsu stops one after 10 minutes. Stop ends it now
            and changes nothing.
          </span>
        )}
      </p>
      {agent && (
        <button
          disabled={stopping}
          onClick={() => {
            setStopping(true);
            onStop();
          }}
        >
          {stopping ? "Stopping…" : "Stop"}
        </button>
      )}
    </div>
  );
}
