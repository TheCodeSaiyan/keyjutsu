import type { ExecutionMode, ExecutionState } from "@keyjutsu/types";

/**
 * Labels for the operator's view. Where the design kit names a state (§11),
 * its canonical label is used so the desktop, CLI and docs agree; the rest
 * are the engine's own states in plain words.
 */
const STATE_LABELS: Record<ExecutionState, string> = {
  PREPARING: "Preparing",
  ARMED: "Armed",
  TYPING: "Typing",
  AWAITING_EXECUTION: "Ready to submit",
  EXECUTING: "Running",
  WAITING: "Waiting",
  VALIDATING: "Validating",
  AWAITING_USER_INPUT: "Waiting for user",
  PAUSED: "Paused",
  FAILED: "Failed",
  REVALIDATION_REQUIRED: "Revalidation required",
  COMPLETE: "Complete",
  ABORTED: "Disarmed",
};

const MODE_LABELS: Record<ExecutionMode, string> = {
  performance: "Performance",
  assisted: "Assisted",
  auto_performance: "Auto performance",
  direct: "Direct",
  user_input: "User input",
};

export const stateLabel = (s: ExecutionState) => STATE_LABELS[s];
export const modeLabel = (m: ExecutionMode) => MODE_LABELS[m];

/** Two-digit step ordinals, as the kit's mockups show them: `05 · Verify recovery`. */
export const ordinal = (index: number) => String(index + 1).padStart(2, "0");
