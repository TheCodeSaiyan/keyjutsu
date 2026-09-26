import type { ShellKind } from "@keyjutsu/types";

/**
 * A staged error: what the terminal shows, with "Stage an error" chosen,
 * while KeyJutsu waits for the operator. To the room it is a command that
 * failed; to the operator, its wording says what KeyJutsu wants:
 *
 * - "…until it is confirmed": a critical step is waiting for its phrase.
 * - "…until the session is resumed": a plan is waiting to continue after
 *   a restart.
 *
 * It is drawn on a layer over the terminal, never into it: the shell and the
 * pseudo-console know nothing of it, so nothing they redraw can clash with
 * it, and it is not in the transcript, the history, or anything an agent is
 * shown. It goes the moment the operator answers.
 */
export type Waiting = "confirm" | "resume";

export interface StagedLine {
  text: string;
  /** Drawn in the shell's error colour. */
  error: boolean;
}

const MESSAGE: Record<Waiting, string> = {
  confirm: "The operation cannot continue until it is confirmed.",
  resume: "The operation cannot continue until the session is resumed.",
};

/**
 * The lines to draw below the prompt, in the style of `shell`'s own errors,
 * ending with a copy of `prompt` so it looks as if the shell is ready again.
 */
export function stagedError(waiting: Waiting, shell: ShellKind, prompt: string): StagedLine[] {
  const message = MESSAGE[waiting];
  const error = (text: string) => ({ text, error: true });
  const plain = (text: string) => ({ text, error: false });
  switch (shell) {
    case "windows_powershell":
      return [
        error(message),
        error("    + CategoryInfo          : OperationStopped: (:) [], RuntimeException"),
        error("    + FullyQualifiedErrorId : OperationStopped"),
        plain(""),
        plain(prompt),
      ];
    case "pwsh":
      return [error(`OperationStopped: ${message}`), plain(prompt)];
    case "cmd":
      return [plain(message), plain(""), plain(prompt)];
  }
}

/** Whether `count` rows fit below row `cursorRow` of `rows` on screen. */
export function fits(cursorRow: number, rows: number, count: number): boolean {
  return cursorRow + count < rows;
}
