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
export function screenRows(term: Terminal, theme: ITheme, area?: Area): Run[][] {
  const buffer = term.buffer.active;
  const fgDefault = theme.foreground ?? "#cccccc";
  const bgDefault = theme.background ?? "#0c0c0c";
  const rows: Run[][] = [];
  const rowCount = Math.min(term.rows, area?.rows ?? term.rows);
  const colCount = Math.min(term.cols, area?.cols ?? term.cols);
  for (let y = 0; y < rowCount; y++) {
    const line = buffer.getLine(buffer.viewportY + y);
    const runs: Run[] = [];
    for (let x = 0; line && x < colCount; x++) {
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

  /** Write everything recorded up to `at` seconds, at the size it was drawn for. */
  async advanceTo(at: number): Promise<void> {
    let data = "";
    const flush = async () => {
      if (data) await new Promise<void>((done) => this.term.write(data, done));
      data = "";
    };
    while (this.next < this.rec.events.length && this.rec.events[this.next].at <= at) {
      const e = this.rec.events[this.next++];
      if (e.kind === "output") data += e.data;
      if (e.kind === "resize") {
        const size = sizeOf(e.data);
        if (size) {
          // What came before was drawn for the old size.
          await flush();
          this.term.resize(size.cols, size.rows);
        }
      }
    }
    await flush();
  }

  dispose(): void {
    this.term.dispose();
  }
}

/** A part of the screen, from the top left, in cells. */
export interface Area {
  cols: number;
  rows: number;
}

/** The size a resize event gives: `COLSxROWS`. */
export function sizeOf(data: string): Area | null {
  const m = /^(\d+)x(\d+)$/.exec(data.trim());
  return m ? { cols: Number(m[1]), rows: Number(m[2]) } : null;
}

/**
 * How much of the screen the recording ever used, and the most it ever had:
 * so an export can be cut to what was drawn on rather than the whole window
 * the terminal happened to fill.
 */
export async function areas(rec: recording.Recording): Promise<{ used: Area; whole: Area }> {
  const player = new Player(rec);
  const whole = { cols: rec.width, rows: rec.height };
  const used = { cols: 1, rows: 1 };
  const look = () => {
    const t = player.term;
    whole.cols = Math.max(whole.cols, t.cols);
    whole.rows = Math.max(whole.rows, t.rows);
    const buffer = t.buffer.active;
    for (let y = 0; y < t.rows; y++) {
      const line = buffer.getLine(buffer.viewportY + y);
      const text = line?.translateToString(true) ?? "";
      if (text.trim()) {
        used.rows = Math.max(used.rows, y + 1);
        used.cols = Math.max(used.cols, text.length);
      }
    }
  };
  // Every tenth of a second, and at the end.
  for (let at = 0; at <= player.duration; at += 0.1) {
    await player.advanceTo(at);
    look();
  }
  await player.advanceTo(Infinity);
  look();
  player.dispose();
  return {
    used: { cols: Math.min(used.cols, whole.cols), rows: Math.min(used.rows, whole.rows) },
    whole,
  };
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

/** The canvas size an area is painted at. */
export function canvasSize(area: Area, m: Metrics): { width: number; height: number } {
  // Video encoders want even sizes.
  const even = (n: number) => n + (n % 2);
  return {
    width: even(area.cols * m.cellWidth + 2 * m.padding),
    height: even(area.rows * m.cellHeight + 2 * m.padding),
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
