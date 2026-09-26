import { describe, expect, it } from "vitest";
import type { KeyChord } from "@keyjutsu/types";
import { PRESENTATIONS, isOperatorChord, rules } from "./presentation";

const chord = (c: Partial<KeyChord> & Pick<KeyChord, "key">): KeyChord => ({
  ctrl: false,
  alt: false,
  shift: false,
  meta: false,
  ...c,
});

describe("presentation", () => {
  it("covers the terminal only when the operator asked to be told on screen", () => {
    expect(rules("standard").coverTerminal).toBe(true);
    expect(rules("discreet").coverTerminal).toBe(false);
    expect(rules("hidden").coverTerminal).toBe(false);
  });

  it("keeps the illusion with a faint edge, or no mark at all", () => {
    expect(rules("discreet")).toMatchObject({ edgeCue: true, flashTaskbar: false });
    expect(rules("hidden")).toMatchObject({ edgeCue: false, flashTaskbar: true });
    expect(rules("standard")).toMatchObject({ edgeCue: false, flashTaskbar: false });
  });

  it("holds the stage after a run whenever the terminal is not covered", () => {
    // Otherwise the screen jumps to the run panel, and mashing goes on into
    // the real shell.
    for (const p of PRESENTATIONS) {
      expect(rules(p.value).holdAfterRun).toBe(!rules(p.value).coverTerminal);
    }
  });

  it("says credential required over the terminal only when showing it", () => {
    expect(rules("standard").credentialBanner).toBe(true);
    expect(rules("discreet").credentialBanner).toBe(false);
    expect(rules("hidden").credentialBanner).toBe(false);
  });

  it("stages an error only when asked to, with nothing else on screen", () => {
    expect(rules("staged")).toMatchObject({
      stagedError: true,
      coverTerminal: false,
      edgeCue: false,
      flashTaskbar: false,
    });
    for (const p of ["standard", "discreet", "hidden"] as const) {
      expect(rules(p).stagedError).toBe(false);
    }
  });

  it("recognises Ctrl+Shift+K and nothing like it", () => {
    expect(isOperatorChord(chord({ key: { char: "K" }, ctrl: true, shift: true }))).toBe(true);
    expect(isOperatorChord(chord({ key: { char: "k" }, ctrl: true, shift: true }))).toBe(true);
    expect(isOperatorChord(chord({ key: { char: "k" }, ctrl: true }))).toBe(false);
    expect(isOperatorChord(chord({ key: { char: "k" }, shift: true }))).toBe(false);
    // Ctrl+Alt+Shift+K is the disarm chord, not this one.
    expect(isOperatorChord(chord({ key: { char: "k" }, ctrl: true, shift: true, alt: true }))).toBe(
      false,
    );
    expect(isOperatorChord(chord({ key: "enter", ctrl: true, shift: true }))).toBe(false);
  });
});
