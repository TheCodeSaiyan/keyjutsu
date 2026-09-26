import type { KeyChord } from "@keyjutsu/types";

/**
 * How KeyJutsu gets the operator's attention during a performance, when it
 * needs them: a critical step to confirm, a restart to continue past, a
 * credential to type, or the end of the run.
 *
 * - `standard`: it says so on screen, over the terminal. Clear, and what
 *   the room sees too.
 * - `discreet`: the terminal is never covered. A faint edge appears on it,
 *   and the question waits in a small corner card the operator opens with
 *   Ctrl+Shift+K. The room sees a terminal that paused.
 * - `hidden`: nothing in the window at all; the taskbar button flashes,
 *   which a shared window does not show. The operator opens the card with
 *   Ctrl+Shift+K.
 *
 * Whatever the choice, nothing runs until the operator answers: this only
 * changes how the question is put.
 */
export type Presentation = "standard" | "discreet" | "hidden";

export const PRESENTATIONS: { value: Presentation; label: string; hint: string }[] = [
  {
    value: "standard",
    label: "Show it",
    hint: "Questions and notices appear over the terminal.",
  },
  {
    value: "discreet",
    label: "Keep the illusion",
    hint: "A faint edge on the terminal; Ctrl+Shift+K opens the question in a corner.",
  },
  {
    value: "hidden",
    label: "Out of view",
    hint: "Only the taskbar button flashes; Ctrl+Shift+K opens the question.",
  },
];

export interface Rules {
  /** A question covers the terminal as soon as it is asked. */
  coverTerminal: boolean;
  /** A faint accent edge on the terminal while KeyJutsu waits. */
  edgeCue: boolean;
  /** Flash the taskbar button while KeyJutsu waits. */
  flashTaskbar: boolean;
  /**
   * After the run, stay on the terminal, with keys held back, until the
   * operator presses the chord: no jump to the run panel for the room to
   * see, and no mashing into the real shell.
   */
  holdAfterRun: boolean;
  /** Say "credential required" over the terminal. */
  credentialBanner: boolean;
}

export function rules(p: Presentation): Rules {
  switch (p) {
    case "standard":
      return {
        coverTerminal: true,
        edgeCue: false,
        flashTaskbar: false,
        holdAfterRun: false,
        credentialBanner: true,
      };
    case "discreet":
      return {
        coverTerminal: false,
        edgeCue: true,
        flashTaskbar: false,
        holdAfterRun: true,
        credentialBanner: false,
      };
    case "hidden":
      return {
        coverTerminal: false,
        edgeCue: false,
        flashTaskbar: true,
        holdAfterRun: true,
        credentialBanner: false,
      };
  }
}

/** Ctrl+Shift+K: the operator's chord, which opens what is waiting. */
export function isOperatorChord(c: KeyChord): boolean {
  return (
    c.ctrl &&
    c.shift &&
    !c.alt &&
    !c.meta &&
    typeof c.key === "object" &&
    "char" in c.key &&
    c.key.char.toLowerCase() === "k"
  );
}

const STORAGE_KEY = "keyjutsu.presentation";

/** The operator's last choice, or `standard`. A convenience: never required. */
export function loadPresentation(): Presentation {
  try {
    const v = localStorage.getItem(STORAGE_KEY);
    if (v === "standard" || v === "discreet" || v === "hidden") return v;
  } catch {
    /* no storage: the default */
  }
  return "standard";
}

export function savePresentation(p: Presentation): void {
  try {
    localStorage.setItem(STORAGE_KEY, p);
  } catch {
    /* not remembered, which is fine */
  }
}
