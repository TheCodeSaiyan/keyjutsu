import type { KeyChord } from "@keyjutsu/types";

type KeyInfo = Pick<KeyboardEvent, "key" | "code" | "ctrlKey" | "altKey" | "shiftKey" | "metaKey">;

const NAMED: Record<string, KeyChord["key"]> = {
  Enter: "enter",
  Tab: "tab",
  Backspace: "backspace",
  Escape: "escape",
  ArrowUp: "up",
  ArrowDown: "down",
  ArrowLeft: "left",
  ArrowRight: "right",
  Home: "home",
  End: "end",
  PageUp: "page_up",
  PageDown: "page_down",
  Insert: "insert",
  Delete: "delete",
};

/**
 * Describe a physical key press for the Rust side, which decides what it
 * means. Returns null for keys that are not a press of anything by
 * themselves: modifiers, dead keys and IME composition.
 */
export function chordFromKeyboardEvent(e: KeyInfo): KeyChord | null {
  const modifiers = { ctrl: e.ctrlKey, alt: e.altKey, shift: e.shiftKey, meta: e.metaKey };
  const named = NAMED[e.key];
  if (named) return { key: named, ...modifiers };

  const f = /^F(\d{1,2})$/.exec(e.key);
  if (f) return { key: { f: Number(f[1]) }, ...modifiers };

  const chars = [...e.key];
  if (chars.length !== 1) return null;

  // With Ctrl, Alt or Win held, browsers report whatever the layout makes of
  // the combination, which may be a control character or nothing useful.
  // The physical letter is what a chord such as Ctrl+Alt+Shift+K means. An
  // AltGr character (Ctrl+Alt producing, say, "@" or "€") is left as typed.
  const letter = /^Key([A-Z])$/.exec(e.code);
  const typedLetter = /^\p{L}$/u.test(e.key) && e.key.toLowerCase() === letter?.[1].toLowerCase();
  const printable = /^[\p{L}\p{N}\p{P}\p{S} ]$/u.test(e.key);
  if (letter && (e.ctrlKey || e.altKey || e.metaKey) && (typedLetter || !printable)) {
    const c = e.shiftKey ? letter[1] : letter[1].toLowerCase();
    return { key: { char: c }, ...modifiers };
  }
  return { key: { char: e.key }, ...modifiers };
}
