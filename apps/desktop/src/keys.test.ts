import { describe, expect, it } from "vitest";
import { chordFromKeyboardEvent } from "./keys";

const press = (
  key: string,
  code: string,
  mods: Partial<Record<"ctrl" | "alt" | "shift" | "meta", boolean>> = {},
) =>
  chordFromKeyboardEvent({
    key,
    code,
    ctrlKey: !!mods.ctrl,
    altKey: !!mods.alt,
    shiftKey: !!mods.shift,
    metaKey: !!mods.meta,
  });

describe("chordFromKeyboardEvent", () => {
  it("reports ordinary characters as typed", () => {
    expect(press("a", "KeyA")).toEqual({
      key: { char: "a" },
      ctrl: false,
      alt: false,
      shift: false,
      meta: false,
    });
    expect(press("A", "KeyA", { shift: true })?.key).toEqual({ char: "A" });
    expect(press(" ", "Space")?.key).toEqual({ char: " " });
    expect(press("é", "Quote")?.key).toEqual({ char: "é" });
  });

  it("reads the disarm chord from the physical key whatever the layout reports", () => {
    // Some layouts report a control character, or another letter, with Ctrl+Alt held.
    for (const key of ["K", "\u000b", "Unidentified"]) {
      const chord = press(key, "KeyK", { ctrl: true, alt: true, shift: true });
      if (key === "Unidentified") {
        expect(chord).toBeNull();
        continue;
      }
      expect(chord).toEqual({
        key: { char: "K" },
        ctrl: true,
        alt: true,
        shift: true,
        meta: false,
      });
    }
  });

  it("keeps AltGr characters as the character produced", () => {
    expect(press("@", "Digit2", { ctrl: true, alt: true })?.key).toEqual({ char: "@" });
    expect(press("€", "KeyE", { ctrl: true, alt: true })?.key).toEqual({ char: "€" });
  });

  it("names editing and navigation keys", () => {
    expect(press("Enter", "Enter")?.key).toBe("enter");
    expect(press("Escape", "Escape")?.key).toBe("escape");
    expect(press("ArrowLeft", "ArrowLeft")?.key).toBe("left");
    expect(press("F12", "F12")?.key).toEqual({ f: 12 });
  });

  it("ignores keys that are not a press of anything on their own", () => {
    for (const key of ["Shift", "Control", "Alt", "Dead", "Process", "CapsLock"]) {
      expect(press(key, key)).toBeNull();
    }
  });
});
