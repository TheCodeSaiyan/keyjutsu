import type { ITerminalOptions, ITheme } from "@xterm/xterm";
import type { TerminalProfile } from "@keyjutsu/types";

/** Windows Terminal sizes fonts in points; xterm.js in CSS pixels. */
export const pointsToPixels = (pt: number) => Math.round((pt * 96) / 72);

export function themeFromProfile(profile: TerminalProfile): ITheme {
  const s = profile.color_scheme;
  const [black, red, green, yellow, blue, magenta, cyan, white, ...bright] = s.ansi;
  return {
    background: s.background,
    foreground: s.foreground,
    cursor: s.cursor,
    cursorAccent: s.background,
    selectionBackground: s.selection_background + "66",
    black,
    red,
    green,
    yellow,
    blue,
    magenta,
    cyan,
    white,
    brightBlack: bright[0],
    brightRed: bright[1],
    brightGreen: bright[2],
    brightYellow: bright[3],
    brightBlue: bright[4],
    brightMagenta: bright[5],
    brightCyan: bright[6],
    brightWhite: bright[7],
  };
}

/** xterm.js options that make the terminal look like the user's own. */
export function optionsFromProfile(profile: TerminalProfile): ITerminalOptions {
  return {
    fontFamily: `"${profile.font_face}", "Cascadia Mono", Consolas, monospace`,
    fontSize: pointsToPixels(profile.font_size),
    cursorStyle: profile.cursor_shape === "underline" ? "underline" : profile.cursor_shape,
    cursorBlink: true,
    theme: themeFromProfile(profile),
  };
}

/** CSS padding for the terminal's container, in Windows Terminal's order. */
export function paddingFromProfile(profile: TerminalProfile): string {
  const [left, top, right, bottom] = profile.padding;
  return `${top}px ${right}px ${bottom}px ${left}px`;
}
