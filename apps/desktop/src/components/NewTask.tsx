import { useState } from "react";
import type { agent, ReadinessReport } from "@keyjutsu/types";

interface Props {
  agents: agent.AgentInfo[] | null;
  report: ReadinessReport | null;
  busy: boolean;
  onPlan(task: string, agent: agent.AgentKind, context: string): void;
  onOpen(text: string): void;
}

/**
 * The first thing on screen (§8): what to do, which agent, and optional
 * context. Nothing is sent until "Plan task", and then only the task and the
 * pasted context, redacted by Rust before it leaves.
 */
export function NewTask({ agents, report, busy, onPlan, onOpen }: Props) {
  const installed = (agents ?? []).filter((a) => a.path !== null);
  const [task, setTask] = useState("");
  const [chosen, setChosen] = useState<agent.AgentKind | "">("");
  const [showContext, setShowContext] = useState(false);
  const [context, setContext] = useState("");
  const agentKind = chosen || installed[0]?.kind;
  const pwsh = report?.shells.find((s) => s.kind === "pwsh");

  return (
    <div className="new-task">
      <p className="eyebrow">New task</p>
      <h1>What do you want KeyJutsu to do?</h1>
      <p className="lede">
        Describe the outcome. The agent investigates without changing anything; then you review
        exactly what can run.
      </p>
      <div className="task-box">
        <textarea
          aria-label="Task"
          className="task-input"
          rows={5}
          value={task}
          onChange={(e) => setTask(e.target.value)}
          placeholder="Docker Desktop will not start after the last update."
        />
        {showContext && (
          <textarea
            aria-label="Context to send with the task"
            rows={5}
            value={context}
            onChange={(e) => setContext(e.target.value)}
            placeholder="Paste an error, a log or a config file. Secrets that look like tokens or passwords are redacted before sending."
          />
        )}
        <div className="task-actions">
          <button onClick={() => setShowContext((v) => !v)} aria-expanded={showContext}>
            {showContext ? "Hide context" : "+ Add context"}
          </button>
          <label className="file-button">
            Open plan file…
            <input
              type="file"
              accept=".json,application/json"
              onChange={async (e) => {
                const file = e.target.files?.[0];
                if (file) onOpen(await file.text());
                e.target.value = "";
              }}
            />
          </label>
          <label className="inline">
            <span className="visually-hidden">Agent</span>
            <select
              value={agentKind ?? ""}
              onChange={(e) => setChosen(e.target.value as agent.AgentKind)}
              disabled={installed.length === 0}
            >
              {installed.length === 0 && <option value="">No agent installed</option>}
              {installed.map((a) => (
                <option key={a.kind} value={a.kind}>
                  {a.name} {a.version ?? ""}
                </option>
              ))}
            </select>
          </label>
          <button
            className="primary"
            disabled={busy || !task.trim() || !agentKind}
            onClick={() => agentKind && onPlan(task.trim(), agentKind, context)}
          >
            {busy ? "Planning…" : "Plan task →"}
          </button>
        </div>
      </div>
      {showContext && (
        <p className="muted small">
          Sent with the task:{" "}
          {context.trim() ? `${context.length} characters of pasted text` : "nothing yet"}. Nothing
          else from this machine is included.
        </p>
      )}
      <ul className="facts" aria-label="This machine">
        {pwsh && <li className="chip">PowerShell {pwsh.version}</li>}
        {report && <li className="chip">{report.windows.product}</li>}
      </ul>
      <div className="cards">
        <section className="card">
          <h2>Agent</h2>
          <p className="small muted">
            {installed.length
              ? installed.map((a) => a.name).join(" · ")
              : "No agent CLI found. You can still open a plan file."}
          </p>
        </section>
        <section className="card">
          <h2>Terminal</h2>
          <p className="small muted">
            ConPTY · {pwsh ? `PowerShell ${pwsh.version}` : "no PowerShell 7"}
          </p>
        </section>
        <section className="card">
          <h2>Execution</h2>
          <p className="small muted">
            Standard user · Administrator steps ask once, through the broker
          </p>
        </section>
        <section className="card">
          <h2>Privacy</h2>
          <p className="small muted">No telemetry · what it keeps stays on this machine</p>
        </section>
      </div>
    </div>
  );
}
