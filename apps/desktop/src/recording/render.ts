import { Terminal } from "@xterm/headless";
import type { ITheme } from "@xterm/xterm";
import type { recording } from "@keyjutsu/types";

/**
 * Replays a recording into a headless terminal and paints what it shows,
 * cell by cell, in the operator's own font and colours (ADR 0021). The
 * recording is the stream the terminal drew, so the replay is exact.
 */

/** One stretch of a row in one style. */
export interface Run {
  text: string;
  fg: string;
  bg: string;
  bold: boolean;
  /** Columns it starts at and spans. */
  x: number;
  width: number;
}

const ANSI_NAMES: (keyof ITheme)[] = [
  "black",
  "red",
  "green",
  "yellow",
  "blue",
  "magenta",
  "cyan",
  "white",
  "brightBlack",
  "brightRed",
  "brightGreen",
  "brightYellow",
  "brightBlue",
  "brightMagenta",
  "brightCyan",
  "brightWhite",
];

const hex = (n: number) => n.toString(16).padStart(2, "0");

/** A colour of the 256-colour palette, the first 16 from the theme. */
export function paletteColour(index: number, theme: ITheme): string {
  if (index < 16) return (theme[ANSI_NAMES[index]] as string | undefined) ?? "#ffffff";
  if (index < 232) {
    const i = index - 16;
    const level = (v: number) => (v === 0 ? 0 : 55 + v * 40);
    return `#${hex(level(Math.floor(i / 36)))}${hex(level(Math.floor(i / 6) % 6))}${hex(level(i % 6))}`;
  }
  const grey = 8 + (index - 232) * 10;
  return `#${hex(grey)}${hex(grey)}${hex(grey)}`;
}

interface Cell {
  getChars(): string;
  getWidth(): number;
  isFgRGB(): boolean;
  isFgPalette(): boolean;
  isBgRGB(): boolean;
  isBgPalette(): boolean;
  getFgColor(): number;
  getBgColor(): number;
  isBold(): number;
  isInverse(): number;
}

function resolve(rgb: boolean, palette: boolean, value: number, theme: ITheme, fallback: string) {
  if (rgb) return `#${hex((value >> 16) & 255)}${hex((value >> 8) & 255)}${hex(value & 255)}`;
  if (palette) return paletteColour(value, theme);
  return fallback;
}

/** The screen as it is now: each row as runs of one style. */
export function screenRows(term: Terminal, theme: ITheme): Run[][] {
  const buffer = term.buffer.active;
  const fgDefault = theme.foreground ?? "#cccccc";
  const bgDefault = theme.background ?? "#0c0c0c";
  const rows: Run[][] = [];
  for (let y = 0; y < term.rows; y++) {
    const line = buffer.getLine(buffer.viewportY + y);
    const runs: Run[] = [];
    for (let x = 0; line && x < term.cols; x++) {
      const cell = line.getCell(x) as Cell | undefined;
      if (!cell || cell.getWidth() === 0) continue;
      let fg = resolve(cell.isFgRGB(), cell.isFgPalette(), cell.getFgColor(), theme, fgDefault);
      let bg = resolve(cell.isBgRGB(), cell.isBgPalette(), cell.getBgColor(), theme, bgDefault);
      if (cell.isInverse()) [fg, bg] = [bg, fg];
      const bold = cell.isBold() !== 0;
      const text = cell.getChars() || " ";
      const last = runs[runs.length - 1];
      if (
        last &&
        last.fg === fg &&
        last.bg === bg &&
        last.bold === bold &&
        last.x + last.width === x
      ) {
        last.text += text;
        last.width += cell.getWidth();
      } else {
        runs.push({ text, fg, bg, bold, x, width: cell.getWidth() });
      }
    }
    rows.push(runs);
  }
  return rows;
}

/** Plays a recording forward into a headless terminal, event by event. */
export class Player {
  readonly term: Terminal;
  private next = 0;

  constructor(private readonly rec: recording.Recording) {
    this.term = new Terminal({
      cols: rec.width,
      rows: rec.height,
      allowProposedApi: true,
      scrollback: 0,
    });
  }

  /** Seconds the recording lasts. */
  get duration(): number {
    return this.rec.events.length ? this.rec.events[this.rec.events.length - 1].at : 0;
  }

  /** Write everything recorded up to `at` seconds. */
  async advanceTo(at: number): Promise<void> {
    let data = "";
    while (this.next < this.rec.events.length && this.rec.events[this.next].at <= at) {
      const e = this.rec.events[this.next++];
      if (e.kind === "output") data += e.data;
    }
    if (data) await new Promise<void>((done) => this.term.write(data, done));
  }

  dispose(): void {
    this.term.dispose();
  }
}

/** The size of one cell, and the font, for painting. */
export interface Metrics {
  font: string;
  boldFont: string;
  cellWidth: number;
  cellHeight: number;
  padding: number;
}

export function measure(ctx: CanvasRenderingContext2D, family: string, sizePx: number): Metrics {
  const font = `${sizePx}px ${family}`;
  ctx.font = font;
  const cellWidth = Math.ceil(ctx.measureText("W").width);
  return {
    font,
    boldFont: `bold ${font}`,
    cellWidth,
    cellHeight: Math.ceil(sizePx * 1.25),
    padding: Math.round(sizePx / 2),
  };
}

/** The canvas size a recording is painted at. */
export function canvasSize(
  rec: recording.Recording,
  m: Metrics,
): { width: number; height: number } {
  // Video encoders want even sizes.
  const even = (n: number) => n + (n % 2);
  return {
    width: even(rec.width * m.cellWidth + 2 * m.padding),
    height: even(rec.height * m.cellHeight + 2 * m.padding),
  };
}

export function paint(
  ctx: CanvasRenderingContext2D,
  rows: Run[][],
  m: Metrics,
  theme: ITheme,
): void {
  ctx.fillStyle = theme.background ?? "#0c0c0c";
  ctx.fillRect(0, 0, ctx.canvas.width, ctx.canvas.height);
  ctx.textBaseline = "top";
  rows.forEach((runs, y) => {
    const top = m.padding + y * m.cellHeight;
    for (const r of runs) {
      const left = m.padding + r.x * m.cellWidth;
      if (r.bg !== (theme.background ?? "#0c0c0c")) {
        ctx.fillStyle = r.bg;
        ctx.fillRect(left, top, r.width * m.cellWidth, m.cellHeight);
      }
      if (r.text.trim()) {
        ctx.font = r.bold ? m.boldFont : m.font;
        ctx.fillStyle = r.fg;
        // Cell by cell, so every character sits in its own column whatever
        // the font's own spacing.
        [...r.text].forEach((c, i) => ctx.fillText(c, left + i * m.cellWidth, top + 2));
      }
    }
  });
}
