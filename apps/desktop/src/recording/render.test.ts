import { describe, expect, it } from "vitest";
import type { recording } from "@keyjutsu/types";
import { paletteColour, Player, screenRows } from "./render";

const theme = {
  background: "#000000",
  foreground: "#cccccc",
  red: "#c50f1f",
  brightGreen: "#16c60c",
};

const rec = (events: [number, string][]): recording.Recording => ({
  width: 20,
  height: 3,
  title: "t",
  events: events.map(([at, data]) => ({ at, kind: "output" as const, data })),
});

describe("the replay", () => {
  it("shows what was drawn up to a moment, in the theme's colours", async () => {
    const p = new Player(
      rec([
        [0, "PS> "],
        [1, "\u001b[31mred\u001b[0m ok"],
        [5, "\r\nlater"],
      ]),
    );
    await p.advanceTo(1);
    const rows = screenRows(p.term, theme);
    const text = rows[0].map((r) => r.text).join("");
    expect(text.trimEnd()).toBe("PS> red ok");
    const red = rows[0].find((r) => r.text === "red");
    expect(red?.fg).toBe("#c50f1f");
    expect(rows[0][0].fg).toBe("#cccccc");
    expect(
      rows[1]
        .map((r) => r.text)
        .join("")
        .trim(),
    ).toBe("");
    await p.advanceTo(5);
    expect(
      screenRows(p.term, theme)[1]
        .map((r) => r.text)
        .join("")
        .trim(),
    ).toBe("later");
    p.dispose();
  });

  it("resolves 256 colours and true colour", async () => {
    expect(paletteColour(10, theme)).toBe("#16c60c");
    expect(paletteColour(196, theme)).toBe("#ff0000");
    expect(paletteColour(244, theme)).toBe("#808080");
    const p = new Player(rec([[0, "\u001b[38;2;1;2;3mx"]]));
    await p.advanceTo(0);
    expect(screenRows(p.term, theme)[0][0].fg).toBe("#010203");
    p.dispose();
  });
});
