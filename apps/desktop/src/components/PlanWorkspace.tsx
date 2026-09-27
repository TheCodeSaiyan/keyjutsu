import { useState } from "react";
import type { agent, plan, workspace, Sealed } from "@keyjutsu/types";
import {
  everValidated,
  formFromStep,
  idFor,
  newStep,
  readinessLabel,
  readinessTone,
  riskLabel,
  stepFromForm,
  summaryLine,
  type StepForm,
  retryGuidance,
  STEP_MODES,
} from "../plan";
import { ordinal } from "../labels";
import { ipc } from "../ipc";

interface Props {
  view: workspace.WorkspaceView;
  agents: agent.AgentInfo[] | null;
  /** What is in progress, if anything, in words ("Validating…"). */
  busy: string | null;
  sealed: Sealed | null;
  canArm: boolean;
  /** Run a request that returns the new view. */
  act(label: string, request: () => Promise<workspace.WorkspaceView>): void;
  onApprove(): void;
  onArm(): void;
  /** How the plan will run: the same choice as the Terminal's Performance panel. */
  mode: string;
  modes: { value: string; label: string; hint: string }[];
  onMode(mode: string): void;
}

/**
 * The plan workspace. The plan is authoritative and in the middle;
 * the agent's notes explain it on the left; the selected step is inspected
 * and edited on the right. Every change is a request to Rust, which re-reads
 * the whole plan and decides what needs validating again.
 */
export function PlanWorkspace(props: Props) {
  const { view, agents, busy, sealed, canArm, act } = props;
  const [selected, setSelected] = useState<string | null>(null);
  const validatedOnce = everValidated(view.plan);
  const installed = (agents ?? []).filter((a) => a.path !== null);
  const [agentKind, setAgentKind] = useState<agent.AgentKind | undefined>(undefined);
  const [saved, setSaved] = useState<string | null>(null);
  const [saveError, setSaveError] = useState<string | null>(null);
  const primary = agentKind ?? installed[0]?.kind;

  // A selection that no longer exists (the step was removed) falls back to
  // the first step.
  const current =
    selected && view.steps.some((s) => s.id === selected) ? selected : (view.steps[0]?.id ?? null);
  const summary = view.steps.find((s) => s.id === current) ?? null;
  const step = view.plan.steps.find((s) => s.id === current) ?? null;
  const affected = new Set(view.last_change?.affected ?? []);

  return (
    <div className="workspace">
      <AgentPanel
        view={view}
        installed={installed}
        primary={primary}
        setPrimary={setAgentKind}
        busy={busy}
        act={act}
      />

      <section className="plan-column" aria-label="Execution plan">
        <header className="plan-head">
          <div>
            <p className="eyebrow muted">Execution plan</p>
            <h1>{view.plan.title ?? view.task}</h1>
          </div>
          <div className="chips">
            <span className="chip">Risk · {riskLabel(view.overall.highest_risk)}</span>
            <span className="chip">Local Windows</span>
          </div>
        </header>

        {view.last_change && view.last_change.affected.length > 0 && (
          <p className="banner" role="status">
            Changed. Revalidation required for{" "}
            {view.last_change.affected
              .map((id) => view.steps.find((s) => s.id === id))
              .filter(Boolean)
              .map((s) => `${ordinal(s!.number - 1)} ${s!.title}`)
              .join(", ")}
            .
          </p>
        )}
        {view.problems.map((p) => (
          <p key={p} className="banner error" role="alert">
            {p}
          </p>
        ))}

        <ol className="plan-list">
          {view.steps.map((s) => (
            <li key={s.id}>
              <button
                className="plan-row"
                aria-current={s.id === current ? "true" : undefined}
                onClick={() => setSelected(s.id)}
              >
                <span className="ordinal">{ordinal(s.number - 1)}</span>
                <span className="row-text">
                  <span className="row-title">{s.title}</span>
                  <span className="row-detail">{s.detail}</span>
                </span>
                <span className="row-badges">
                  {s.concerns > 0 && (
                    <span className="concern" title="A reviewer raised a concern">
                      {s.concerns} concern{s.concerns > 1 ? "s" : ""}
                    </span>
                  )}
                  {s.critical && <span className="badge critical">Critical</span>}
                  <span
                    className={`badge ${readinessTone(s.readiness)}`}
                    data-affected={affected.has(s.id)}
                  >
                    {readinessLabel(s.readiness, validatedOnce)}
                  </span>
                </span>
                <span className="risk">{s.risk ? riskLabel(s.risk) : ""}</span>
              </button>
            </li>
          ))}
        </ol>

        <AddStep view={view} after={current} act={act} busy={busy} />

        <footer className="plan-foot">
          <div className="foot-facts">
            <span>
              <strong>{summaryLine(view.overall)}</strong>
            </span>
            <span className="muted">
              {view.overall.needs_elevation ? "Elevation needed" : "No elevation"}
            </span>
            <span className="muted">
              {view.overall.recovery_prepared ? "Recovery: none missing" : "Some recovery missing"}
            </span>
          </div>
          <div className="row">
            {view.plan.steps.some((s) => s.artifacts.length > 0) && (
              <button
                disabled={busy !== null}
                title="Download each artifact now, check it against its pinned hash and keep it for the run"
                onClick={() => act("Downloading and checking artifacts…", () => ipc.stage())}
              >
                Stage downloads
              </button>
            )}
            <button
              disabled={busy !== null}
              title="Save this plan, with what validation found for every step, to keep or to hand to someone helping"
              onClick={() => {
                setSaveError(null);
                ipc.savePlan().then(setSaved, (e: unknown) => setSaveError(String(e)));
              }}
            >
              Save plan
            </button>
            <button
              disabled={busy !== null}
              onClick={() => act("Validating…", () => ipc.validate())}
            >
              Validate
            </button>
            <label
              className="small run-mode"
              title={props.modes.find((m) => m.value === props.mode)?.hint}
            >
              Runs in{" "}
              <select
                value={props.mode}
                disabled={busy !== null}
                onChange={(e) => props.onMode(e.target.value)}
              >
                {props.modes.map((m) => (
                  <option key={m.value} value={m.value}>
                    {m.label}
                  </option>
                ))}
              </select>
            </label>
            {sealed ? (
              <button
                className="primary arm"
                disabled={!canArm || busy !== null}
                onClick={props.onArm}
              >
                Arm KeyJutsu
              </button>
            ) : (
              <button
                className="primary"
                disabled={busy !== null || view.overall.blocking.length > 0}
                title={view.overall.blocking.join("; ")}
                onClick={props.onApprove}
              >
                Approve plan
              </button>
            )}
          </div>
          {view.overall.blocking.length > 0 && (
            <p className="muted small">Before approval: {view.overall.blocking.join("; ")}.</p>
          )}
          {sealed && (
            <p className="muted small">
              Approved and sealed. Saved to {sealed.path}, so <code>keyjutsu run</code> and{" "}
              <code>keyjutsu recover</code> can use it too.
            </p>
          )}
        </footer>
        {saved && (
          <p role="status" className="small">
            Saved to <code>{saved}</code>, with what validation found for every step. Open it again
            with Open plan file…. It holds the plan&apos;s commands and paths: read it before you
            send it to anyone.
          </p>
        )}
        {saveError && (
          <p role="alert" className="error">
            {saveError}
          </p>
        )}
      </section>

      <section className="step-column" aria-label="Selected step">
        {summary && step ? (
          <StepPanel
            key={step.id}
            view={view}
            summary={summary}
            step={step}
            validatedOnce={validatedOnce}
            primary={primary}
            busy={busy}
            act={act}
          />
        ) : (
          <p className="muted">Select a step to see what it does.</p>
        )}
      </section>
    </div>
  );
}

function AgentPanel({
  view,
  installed,
  primary,
  setPrimary,
  busy,
  act,
}: {
  view: workspace.WorkspaceView;
  installed: agent.AgentInfo[];
  primary: agent.AgentKind | undefined;
  setPrimary(k: agent.AgentKind): void;
  busy: string | null;
  act: Props["act"];
}) {
  const [guidance, setGuidance] = useState("");
  const reviewers = installed.filter((a) => a.kind !== primary);
  const [reviewer, setReviewer] = useState<agent.AgentKind | undefined>(undefined);
  const chosenReviewer = reviewer ?? reviewers[0]?.kind;
  const numberOf = (id?: string) => view.steps.find((s) => s.id === id)?.number;

  return (
    <section className="agent-column" aria-label="Agent">
      <header className="column-head">
        <h2>Agent</h2>
        <span className="chip">Primary</span>
      </header>
      <label className="small">
        <span className="visually-hidden">Primary agent</span>
        <select
          aria-label="Primary agent"
          value={primary ?? ""}
          onChange={(e) => setPrimary(e.target.value as agent.AgentKind)}
          disabled={installed.length === 0}
        >
          {installed.length === 0 && <option value="">None installed</option>}
          {installed.map((a) => (
            <option key={a.kind} value={a.kind}>
              {a.name}
            </option>
          ))}
        </select>
      </label>
      <ol className="notes">
        {view.notes.length === 0 && (
          <li className="muted small">No conversation yet: this plan was opened from a file.</li>
        )}
        {view.notes.map((n, i) => (
          <li key={i} className={n.who === "You" ? "note you" : n.review ? "note review" : "note"}>
            <strong>
              {n.who}
              {n.step && numberOf(n.step) !== undefined
                ? ` · step ${ordinal(numberOf(n.step)! - 1)}`
                : ""}
            </strong>
            <span>{n.text}</span>
          </li>
        ))}
      </ol>
      <label className="small">
        Guidance
        <textarea
          rows={3}
          value={guidance}
          onChange={(e) => setGuidance(e.target.value)}
          placeholder="Don't replace the whole file; patch the one property."
        />
      </label>
      <div className="stack">
        <button
          disabled={busy !== null || !primary || !guidance.trim()}
          onClick={() =>
            act("The agent is reconsidering the plan…", () => ipc.revise(primary!, guidance.trim()))
          }
        >
          Ask agent to reconsider the plan
        </button>
        <button
          disabled={busy !== null || !guidance.trim()}
          onClick={() => {
            act("Saving note…", () => ipc.note(guidance.trim(), null));
            setGuidance("");
          }}
        >
          Keep as a note
        </button>
        <label className="small">
          Reviewer
          <select
            aria-label="Reviewer"
            value={chosenReviewer ?? ""}
            onChange={(e) => setReviewer(e.target.value as agent.AgentKind)}
            disabled={reviewers.length === 0}
          >
            {reviewers.length === 0 && <option value="">No second agent</option>}
            {reviewers.map((a) => (
              <option key={a.kind} value={a.kind}>
                {a.name} · Reviewer
              </option>
            ))}
          </select>
        </label>
        <button
          disabled={busy !== null || !chosenReviewer}
          onClick={() => act("Asking for a review…", () => ipc.review(chosenReviewer!))}
        >
          Ask for review
        </button>
      </div>
    </section>
  );
}

function evidenceGlyph(result: plan.EvidenceResult): string {
  return result === "passed" ? "✓" : result === "failed" ? "✗" : "–";
}

function StepPanel({
  view,
  summary,
  step,
  validatedOnce,
  primary,
  busy,
  act,
}: {
  view: workspace.WorkspaceView;
  summary: workspace.StepSummary;
  step: plan.Step;
  validatedOnce: boolean;
  primary: agent.AgentKind | undefined;
  busy: string | null;
  act: Props["act"];
}) {
  const [editing, setEditing] = useState(false);
  const [form, setForm] = useState<StepForm>(() => formFromStep(step));
  const [retrying, setRetrying] = useState(false);
  const [guidance, setGuidance] = useState("");
  const state = view.plan.keyjutsu?.steps[step.id];
  const concerns = view.notes.filter((n) => n.review && n.step === step.id);

  if (editing) {
    return (
      <form
        className="step-editor"
        onSubmit={(e) => {
          e.preventDefault();
          act("Checking the change…", () => ipc.replaceStep(stepFromForm(step, form)));
          setEditing(false);
        }}
      >
        <header className="column-head">
          <h2>Edit step {ordinal(summary.number - 1)}</h2>
        </header>
        <label className="small">
          Title
          <input value={form.title} onChange={(e) => setForm({ ...form, title: e.target.value })} />
        </label>
        <label className="small">
          Objective
          <input
            value={form.objective}
            onChange={(e) => setForm({ ...form, objective: e.target.value })}
          />
        </label>
        {step.kind !== "manual" && step.kind !== "credential" && (
          <label className="small">
            Commands, one per line, exactly as they will be typed
            <textarea
              rows={5}
              spellCheck={false}
              value={form.commands}
              onChange={(e) => setForm({ ...form, commands: e.target.value })}
            />
          </label>
        )}
        <label className="small">
          Visible validation, run after it
          <textarea
            rows={2}
            spellCheck={false}
            value={form.visibleValidation}
            onChange={(e) => setForm({ ...form, visibleValidation: e.target.value })}
          />
        </label>
        {step.recovery?.strategy !== "restore_captured_state" && (
          <label className="small">
            Recovery commands, to undo it
            <textarea
              rows={2}
              spellCheck={false}
              value={form.recoveryCommands}
              onChange={(e) => setForm({ ...form, recoveryCommands: e.target.value })}
            />
          </label>
        )}
        <label className="small">
          How this step runs
          <select
            value={form.mode}
            onChange={(e) => setForm({ ...form, mode: e.target.value as StepForm["mode"] })}
          >
            {STEP_MODES.map((m) => (
              <option key={m.value} value={m.value}>
                {m.label}
              </option>
            ))}
          </select>
        </label>
        <p className="muted small">
          Saving sends this step and everything after it back to validation. Approval is never
          carried over.
        </p>
        <div className="row">
          <button type="submit" className="secondary-strong">
            Save
          </button>
          <button type="button" onClick={() => setEditing(false)}>
            Cancel
          </button>
        </div>
      </form>
    );
  }

  return (
    <div className="step-panel">
      <header className="column-head">
        <h2>Step {ordinal(summary.number - 1)}</h2>
        <span className={`badge ${readinessTone(summary.readiness)}`}>
          {readinessLabel(summary.readiness, validatedOnce)}
        </span>
      </header>
      <p className="eyebrow muted">Objective</p>
      <p className="objective">{step.objective}</p>
      {step.reason && <p className="small muted">{step.reason}</p>}

      {step.commands.length > 0 && (
        <>
          <p className="eyebrow muted">Command{step.commands.length > 1 ? "s" : ""}</p>
          <pre className="code">{step.commands.map((c) => c.text).join("\n")}</pre>
        </>
      )}
      {step.execution_mode && (
        <p className="small muted">
          This step always runs as: {STEP_MODES.find((m) => m.value === step.execution_mode)?.label}
        </p>
      )}
      {step.kind === "manual" && (
        <p className="small">You do this yourself, outside the terminal, then press Enter.</p>
      )}
      {step.credential && (
        <p className="small">
          Asks you for {step.credential.kind === "secret" ? "a secret" : "a user name and password"}
          : “{step.credential.prompt}”. You type it into PowerShell’s own masked prompt; KeyJutsu
          never sees it.
        </p>
      )}

      <p className="eyebrow muted">Readiness</p>
      {state ? (
        <ul className="evidence">
          {state.evidence.map((e, i) => (
            <li key={i} data-result={e.result}>
              <span aria-hidden="true">{evidenceGlyph(e.result)}</span> {e.check}
              {e.detail ? `: ${e.detail}` : ""}
              <span className="visually-hidden"> ({e.result})</span>
            </li>
          ))}
          {state.remaining_uncertainty.map((u) => (
            <li key={u} data-result="not_applicable">
              <span aria-hidden="true">?</span> {u}
            </li>
          ))}
          <li className="muted">Proof: {state.proof_level.toLowerCase()}</li>
        </ul>
      ) : (
        <p className="small muted">
          {validatedOnce ? "Changed since validation: validate again." : "Not validated yet."}
        </p>
      )}

      <p className="eyebrow muted">Risk</p>
      <p className="small">
        {riskLabel(summary.risk)}
        {summary.critical ? " · critical action: approval needs its typed confirmation" : ""}
      </p>
      {state && state.risk_reasons.length > 0 && (
        <ul className="small plain">
          {state.risk_reasons.map((r) => (
            <li key={r}>{r}</li>
          ))}
        </ul>
      )}
      {state?.assessed_risk &&
        state.evidence.some((e) => e.check === "risk" && e.result === "failed") && (
          <div className="row">
            <button
              disabled={busy !== null}
              onClick={() =>
                act("Using KeyJutsu's rating…", () =>
                  ipc.replaceStep({
                    ...step,
                    proposed_risk: {
                      level: state.assessed_risk!,
                      rationale: `KeyJutsu's rating, accepted by the operator: ${state.risk_reasons.join("; ")}`,
                    },
                  }),
                )
              }
            >
              Use KeyJutsu&apos;s rating
            </button>
            <span className="small muted">
              The agent rated it lower. This records that you accept KeyJutsu&apos;s rating, then it
              is validated again.
            </span>
          </div>
        )}

      <p className="eyebrow muted">Recovery</p>
      <p className="small">
        {step.recovery
          ? step.recovery.strategy === "restore_captured_state"
            ? `Captures ${step.recovery.capture.map((c) => c.target).join(", ")} first, and can restore it.`
            : step.recovery.strategy === "commands"
              ? "Runs its recovery commands:"
              : "Cannot be undone."
          : "None declared."}
        {step.reversibility?.notes ? ` ${step.reversibility.notes}` : ""}
      </p>
      {step.recovery?.strategy === "commands" && (
        <pre className="code">{step.recovery.commands.map((c) => c.text).join("\n")}</pre>
      )}

      {concerns.length > 0 && (
        <>
          <p className="eyebrow muted">Review</p>
          <ul className="small plain">
            {concerns.map((c, i) => (
              <li key={i}>
                <strong>{c.who}:</strong> {c.text}
              </li>
            ))}
          </ul>
        </>
      )}

      {retrying ? (
        <div className="retry">
          <label className="small">
            What should change?
            <textarea rows={3} value={guidance} onChange={(e) => setGuidance(e.target.value)} />
          </label>
          <p className="muted small">
            The agent gets the task, the plan, this guidance and what validation and review found
            about this step. Only this step can change, and it comes back unvalidated and
            unapproved.
          </p>
          <div className="row">
            <button
              disabled={busy !== null || !primary || !guidance.trim()}
              onClick={() => {
                act("The agent is revising this step…", () =>
                  ipc.retryStep(primary!, step.id, guidance.trim()),
                );
                setRetrying(false);
              }}
            >
              Send to agent
            </button>
            <button onClick={() => setRetrying(false)}>Cancel</button>
          </div>
        </div>
      ) : (
        <div className="stack">
          <button disabled={busy !== null} onClick={() => setEditing(true)}>
            Edit
          </button>
          {step.artifacts.some((a) => !a.sha256) && (
            <button
              disabled={busy !== null}
              title="Download what this step needs once, record its hash, and keep it for the run"
              onClick={() => act("Downloading and checking artifacts…", () => ipc.stage())}
            >
              Stage downloads
            </button>
          )}
          <button
            disabled={busy !== null || !primary}
            onClick={() => {
              setGuidance(retryGuidance(state));
              setRetrying(true);
            }}
          >
            Retry step with agent
          </button>
          <div className="row">
            <button
              disabled={busy !== null}
              onClick={() => act("Moving…", () => ipc.moveStep(step.id, true))}
            >
              Move up
            </button>
            <button
              disabled={busy !== null}
              onClick={() => act("Moving…", () => ipc.moveStep(step.id, false))}
            >
              Move down
            </button>
            <button
              className="text-danger"
              disabled={busy !== null}
              onClick={() => act("Removing…", () => ipc.removeStep(step.id))}
            >
              Remove
            </button>
          </div>
        </div>
      )}
    </div>
  );
}

function AddStep({
  view,
  after,
  act,
  busy,
}: {
  view: workspace.WorkspaceView;
  after: string | null;
  act: Props["act"];
  busy: string | null;
}) {
  const [open, setOpen] = useState(false);
  const [kind, setKind] = useState<"manual" | "validation" | "command">("manual");
  const [title, setTitle] = useState("");
  const [objective, setObjective] = useState("");
  const [commands, setCommands] = useState("");
  const shell = view.plan.steps.find((s) => s.shell)?.shell ?? { kind: "pwsh" as const };
  const afterTitle = view.steps.find((s) => s.id === after)?.title;

  if (!open) {
    return (
      <div className="add-step">
        <button disabled={busy !== null} onClick={() => setOpen(true)}>
          + Add a step{afterTitle ? ` after “${afterTitle}”` : ""}
        </button>
      </div>
    );
  }
  return (
    <form
      className="add-step form"
      onSubmit={(e) => {
        e.preventDefault();
        const id = idFor(
          title,
          view.plan.steps.map((s) => s.id),
        );
        const s = newStep(kind, id, title, objective, commands, shell);
        act("Adding the step…", () => ipc.insertStep(after, s));
        setOpen(false);
        setTitle("");
        setObjective("");
        setCommands("");
      }}
    >
      <div className="row">
        <select
          aria-label="Kind of step"
          value={kind}
          onChange={(e) => setKind(e.target.value as typeof kind)}
        >
          <option value="manual">Something I do myself</option>
          <option value="validation">A check</option>
          <option value="command">A command</option>
        </select>
        <input
          aria-label="Title"
          placeholder="Title"
          value={title}
          onChange={(e) => setTitle(e.target.value)}
          required
        />
      </div>
      <input
        aria-label="Objective"
        placeholder="What it achieves"
        value={objective}
        onChange={(e) => setObjective(e.target.value)}
        required
      />
      {kind !== "manual" && (
        <textarea
          aria-label="Commands, one per line"
          rows={2}
          spellCheck={false}
          value={commands}
          onChange={(e) => setCommands(e.target.value)}
          placeholder="Get-Service -Name docker"
          required
        />
      )}
      <div className="row">
        <button type="submit">Add step</button>
        <button type="button" onClick={() => setOpen(false)}>
          Cancel
        </button>
      </div>
    </form>
  );
}
