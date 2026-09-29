import type { execute, plan, workspace } from "@keyjutsu/types";

/**
 * Helpers for the plan workspace. None of them decides anything: the draft,
 * its validation and every rule about it live in Rust. These turn what Rust
 * sends into words, and the operator's form back into a step for Rust to
 * check.
 */

/** The kit's canonical state labels, or "Validate" before validation. */
export function readinessLabel(r: plan.Readiness | undefined, everValidated: boolean): string {
  switch (r) {
    case "READY":
      return "Ready";
    case "NEEDS_REVIEW":
      return "Review";
    case "BLOCKED":
      return "Blocked";
    case "INVALID":
      return "Invalid";
    case "REVALIDATION_REQUIRED":
      return "Revalidation required";
    case undefined:
      // A step that was validated once and then changed says so; a step in a
      // plan that was never validated just needs it.
      return everValidated ? "Revalidation required" : "Not validated";
  }
}

/** A token for styling; the label is always shown too (colour is never alone). */
export function readinessTone(
  r: plan.Readiness | undefined,
): "ready" | "review" | "blocked" | "none" {
  switch (r) {
    case "READY":
      return "ready";
    case "NEEDS_REVIEW":
      return "review";
    case "BLOCKED":
    case "INVALID":
      return "blocked";
    default:
      return "none";
  }
}

export function riskLabel(r: plan.RiskLevel | undefined): string {
  return r ? r[0].toUpperCase() + r.slice(1) : "Not assessed";
}

export function everValidated(p: plan.Plan): boolean {
  return p.keyjutsu?.provenance?.some((e) => e.action === "validated") ?? false;
}

/** `4/5 READY`, and what stands between the plan and approval. */
export function summaryLine(o: workspace.Overall): string {
  return `${o.ready}/${o.total} ready`;
}

/** One command per non-empty line. Blank lines are dropped; text is kept as typed. */
export function linesToCommands(text: string): plan.Command[] {
  return text
    .split(/\r?\n/)
    .filter((l) => l.trim() !== "")
    .map((text) => ({ text }));
}

export function commandsToLines(commands: plan.Command[]): string {
  return commands.map((c) => c.text).join("\n");
}

/** What the step editor lets the operator change. */
/** How one step runs: the run's mode, or one of its own. */
export const STEP_MODES: { value: "" | plan.ExecutionMode; label: string }[] = [
  { value: "", label: "As the run goes" },
  { value: "performance", label: "Performance" },
  { value: "assisted", label: "Assisted" },
  { value: "auto_performance", label: "Auto" },
  { value: "direct", label: "Direct" },
  { value: "user_input", label: "You type it yourself" },
];

export interface StepForm {
  title: string;
  objective: string;
  commands: string;
  visibleValidation: string;
  recoveryCommands: string;
  /** "" for the run's mode. */
  mode: "" | plan.ExecutionMode;
}

export function formFromStep(s: plan.Step): StepForm {
  return {
    title: s.title,
    objective: s.objective,
    commands: commandsToLines(s.commands),
    visibleValidation: commandsToLines(s.visible_validation),
    recoveryCommands:
      s.recovery?.strategy === "commands" ? commandsToLines(s.recovery.commands) : "",
    mode: s.execution_mode ?? "",
  };
}

/**
 * The step with the operator's edits applied. Everything the form does not
 * show is kept as it was. Recovery commands replace a commands recovery, or
 * add one where the step had none; a captured-state recovery is left alone.
 */
export function stepFromForm(s: plan.Step, f: StepForm): plan.Step {
  const next: plan.Step = {
    ...s,
    title: f.title.trim(),
    objective: f.objective.trim(),
    commands: linesToCommands(f.commands),
    visible_validation: linesToCommands(f.visibleValidation),
    execution_mode: f.mode === "" ? undefined : f.mode,
  };
  const recovery = linesToCommands(f.recoveryCommands);
  if (s.recovery?.strategy === "commands" || (!s.recovery && recovery.length > 0)) {
    next.recovery = recovery.length
      ? {
          strategy: "commands",
          capture: [],
          commands: recovery,
          validation: s.recovery?.validation ?? [],
          notes: s.recovery?.notes,
        }
      : undefined;
  }
  return next;
}

/** A new operator step: `manual` (done outside the terminal) or `validation`. */
export function newStep(
  kind: "manual" | "validation" | "command",
  id: string,
  title: string,
  objective: string,
  commands: string,
  shell: plan.ShellBinding | undefined,
): plan.Step {
  return {
    id,
    title: title.trim(),
    objective: objective.trim(),
    kind,
    shell: kind === "manual" ? undefined : shell,
    commands: kind === "manual" ? [] : linesToCommands(commands),
    tool_requirements: [],
    depends_on: [],
    preconditions: [],
    expected_effects: [],
    visible_validation: [],
    internal_validation: [],
    artifacts: [],
  };
}

/** A step id from a title that is not already taken: `check-the-tray`, `check-the-tray-2`. */
export function idFor(title: string, taken: string[]): string {
  const base =
    title
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, "-")
      .replace(/^-+|-+$/g, "")
      .slice(0, 48) || "step";
  const clean = /^[a-z0-9]/.test(base) ? base : `s-${base}`;
  let id = clean;
  for (let n = 2; taken.includes(id); n++) id = `${clean}-${n}`;
  return id;
}

/** The outcome of a run, in a sentence. */
/** A session boundary as a person would say it. */
export function boundaryName(b: plan.Boundary): string {
  switch (b) {
    case "windows_restart":
      return "Windows restart";
    case "sign_out":
      return "sign-out and sign-in";
    case "shell_restart":
      return "shell restart";
    case "wsl_restart":
      return "WSL restart";
    case "docker_restart":
      return "Docker restart";
  }
}

/**
 * What to do at a boundary to continue the plan. A restart or sign-out
 * closes the app, which offers to continue when it is opened again; the
 * others happen while it stays open.
 */
export function boundaryStep(b: plan.Boundary): string {
  switch (b) {
    case "windows_restart":
      return "Restart Windows, then open KeyJutsu: it offers to continue the plan.";
    case "sign_out":
      return "Sign out of Windows and back in, then open KeyJutsu: it offers to continue the plan.";
    case "shell_restart":
      return "Open a new terminal, then arm the plan again to continue it.";
    case "wsl_restart":
      return "Restart WSL (wsl --shutdown), then arm the plan again to continue it.";
    case "docker_restart":
      return "Restart Docker, then arm the plan again to continue it.";
  }
}

export function outcomeLine(o: { kind: string } & Record<string, unknown>): string {
  switch (o.kind) {
    case "complete":
      return "Complete: every step ran and passed its checks.";
    case "failed":
      return `Step ${String(o.step)} failed. Expected ${String(o.expected)}; got ${String(o.actual)}. Nothing after it ran.`;
    case "aborted":
      return o.in_doubt
        ? `Disarmed during ${String(o.step)} after its command was sent, so its effect is unknown.`
        : "Disarmed. Nothing further ran.";
    case "blocked":
      return `Stopped: ${String(o.reason)}.`;
    case "boundary":
      return `Phase ${String(o.phase)} is done. The plan waits for a ${boundaryName(o.boundary as plan.Boundary)}.`;
    default:
      return "Finished.";
  }
}

/**
 * The confirmation shown at approval for a critical step, built from the plan
 * and its validation. The same shape Rust sends before the step runs, so the
 * operator sees the same thing both times.
 */
export function confirmationFor(
  view: workspace.WorkspaceView,
  id: string,
): execute.CriticalConfirmation | null {
  const step = view.plan.steps.find((s) => s.id === id);
  const summary = view.steps.find((s) => s.id === id);
  if (!step || !summary?.confirmation_phrase) return null;
  const state = view.plan.keyjutsu?.steps[id];
  const impact = [
    ...(step.proposed_risk ? [step.proposed_risk.rationale] : []),
    ...(step.reason ? [step.reason] : []),
    ...(state?.risk_reasons ?? []),
  ];
  const recovery =
    step.recovery?.strategy === "restore_captured_state"
      ? "KeyJutsu captures the current state first and can restore it"
      : step.recovery?.strategy === "commands"
        ? "the plan has recovery commands"
        : step.reversibility?.level === "none"
          ? `NONE: this cannot be undone by KeyJutsu${step.reversibility.notes ? `. ${step.reversibility.notes}` : ""}`
          : "none declared: KeyJutsu cannot undo this";
  return {
    step: id,
    title: step.title,
    commands: step.commands.map((c) => c.text),
    targets: step.expected_effects.map((e) => e.target),
    impact,
    recovery,
    evidence: (state?.evidence ?? []).map((e) => `${e.check}: ${e.detail ?? ""}`),
    phrase: summary.confirmation_phrase,
  };
}

/**
 * What to tell the agent when retrying a step, written from what validation
 * found, so the operator can send it as it is or say more.
 */
export function retryGuidance(state: plan.StepState | undefined): string {
  const found = (state?.evidence ?? [])
    .filter((e) => e.result === "failed" && e.check !== "artifact")
    .map((e) => `- ${e.check}: ${e.detail ?? ""}`);
  if (found.length === 0) return "";
  return `Validation found this step won't run as it is:\n${[...new Set(found)].join("\n")}\nChange it so it will, without changing what it achieves.`;
}

/**
 * A step's evidence as the operator reads it: what failed or could not be
 * decided first, since that is what needs them, and what passed apart, once
 * each, since a plan with many commands passes the same check many times.
 */
export function groupEvidence(evidence: plan.Evidence[]): {
  attention: plan.Evidence[];
  passed: plan.Evidence[];
} {
  const attention = evidence.filter((e) => e.result !== "passed");
  const seen = new Set<string>();
  const passed = evidence.filter((e) => {
    if (e.result !== "passed") return false;
    const key = `${e.check}\u0000${e.detail ?? ""}`;
    if (seen.has(key)) return false;
    seen.add(key);
    return true;
  });
  return { attention, passed };
}
