import { describe, expect, it } from "vitest";
import type { TerminalProfile } from "@keyjutsu/types";
import { optionsFromProfile, paddingFromProfile, pointsToPixels } from "./theme";

const profile: TerminalProfile = {
  source: "fixture",
  name: "PowerShell",
  shell: "pwsh",
  commandline: null,
  font_face: "JetBrains Mono",
  font_size: 11,
  color_scheme: {
    name: "Dusk",
    background: "#1B1D23",
    foreground: "#D8DEE9",
    cursor: "#E5E9F0",
    selection_background: "#434C5E",
    ansi: Array.from({ length: 16 }, (_, i) => `#0000${i.toString(16).padStart(2, "0")}`),
  },
  cursor_shape: "block",
  padding: [6, 4, 2, 8],
  starting_directory: null,
};

describe("profile matching", () => {
  it("converts Windows Terminal points to CSS pixels", () => {
    expect(pointsToPixels(12)).toBe(16);
    expect(pointsToPixels(11)).toBe(15);
  });

  it("maps the scheme's sixteen colours in Windows Terminal's order", () => {
    const theme = optionsFromProfile(profile).theme!;
    expect(theme.black).toBe("#000000");
    expect(theme.magenta).toBe("#000005");
    expect(theme.brightBlack).toBe("#000008");
    expect(theme.brightWhite).toBe("#00000f");
    expect(theme.background).toBe("#1B1D23");
  });

  it("carries font, cursor and padding across", () => {
    const options = optionsFromProfile(profile);
    expect(options.fontFamily).toContain('"JetBrains Mono"');
    expect(options.cursorStyle).toBe("block");
    expect(paddingFromProfile(profile)).toBe("4px 2px 8px 6px");
  });
});
