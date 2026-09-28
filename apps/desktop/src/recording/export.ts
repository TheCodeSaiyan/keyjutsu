import { applyPalette, GIFEncoder, quantize } from "gifenc";
import type { ITheme } from "@xterm/xterm";
import type { recording } from "@keyjutsu/types";
import { canvasSize, measure, paint, Player, screenRows, type Area, type Metrics } from "./render";

/**
 * Turning a prepared recording into files (ADR 0021): a soundless video, a
 * GIF, and a step-by-step guide as Markdown and as one HTML page. The cut,
 * the redaction and the shortened pauses were done in Rust; this only draws.
 */

/** The best soundless video the browser engine can record: MP4 if it can. */
export function videoFormat(
  supported: (mime: string) => boolean,
): { mime: string; extension: string } | null {
  for (const [mime, extension] of [
    ["video/mp4;codecs=avc1", "mp4"],
    ["video/mp4", "mp4"],
    ["video/webm;codecs=vp9", "webm"],
    ["video/webm", "webm"],
  ] as const) {
    if (supported(mime)) return { mime, extension };
  }
  return null;
}

/** A file name for step `n` (from 1): `03-open-pdf`. */
export function stepFileBase(n: number, step: string): string {
  return `${String(n).padStart(2, "0")}-${step.replace(/[^a-z0-9_-]/gi, "-")}`;
}

/** Moments to draw a frame at: every `1/fps` seconds, and the end. */
export function frameTimes(duration: number, fps: number): number[] {
  const times: number[] = [];
  for (let t = 0; t < duration; t += 1 / fps) times.push(Math.round(t * 1000) / 1000);
  times.push(duration);
  return times;
}

const fence = (text: string) => {
  // A fence longer than any run of backticks inside, so none can close it.
  const longest = Math.max(2, ...[...text.matchAll(/`+/g)].map((m) => m[0].length));
  return "`".repeat(longest + 1);
};

/** The guide as Markdown. `images` holds each step's picture's file name. */
export function guideMarkdown(e: recording.Export, images: Map<string, string>): string {
  const out = [`# ${e.title}`, ""];
  e.steps.forEach((s, i) => {
    out.push(`## ${i + 1}. ${s.title}`, "");
    if (s.objective) out.push(s.objective, "");
    if (s.reason) out.push(`> ${s.reason.replace(/\n/g, "\n> ")}`, "");
    if (s.commands.length) {
      const cmd = s.commands.join("\n");
      out.push(`${fence(cmd)}powershell`, cmd, fence(cmd), "");
    }
    const image = images.get(s.step);
    if (image) out.push(`![Step ${i + 1}: ${s.title}](${image})`, "");
    if (s.printed.trim()) {
      out.push(
        "What it printed:",
        "",
        `${fence(s.printed)}text`,
        s.printed.trimEnd(),
        fence(s.printed),
        "",
      );
    }
    if (s.succeeded === false) out.push("**This step failed.**", "");
  });
  if (e.redactions.length) out.push(`_Taken out as secrets: ${e.redactions.join(", ")}._`, "");
  return out.join("\n");
}

export function escapeHtml(text: string): string {
  return text.replace(
    /[&<>"']/g,
    (c) => `&${{ "&": "amp", "<": "lt", ">": "gt", '"': "quot", "'": "#39" }[c]};`,
  );
}

/** The guide as one HTML page. `images` holds each step's picture as a data URI. */
export function guideHtml(e: recording.Export, images: Map<string, string>): string {
  const steps = e.steps
    .map((s, i) => {
      const parts = [`<h2>${i + 1}. ${escapeHtml(s.title)}</h2>`];
      if (s.objective) parts.push(`<p>${escapeHtml(s.objective)}</p>`);
      if (s.reason) parts.push(`<blockquote>${escapeHtml(s.reason)}</blockquote>`);
      if (s.commands.length)
        parts.push(`<pre class="cmd">${escapeHtml(s.commands.join("\n"))}</pre>`);
      const image = images.get(s.step);
      if (image) parts.push(`<img src="${image}" alt="Step ${i + 1}: ${escapeHtml(s.title)}">`);
      if (s.printed.trim()) {
        parts.push(`<p>What it printed:</p><pre>${escapeHtml(s.printed.trimEnd())}</pre>`);
      }
      if (s.succeeded === false) parts.push("<p><strong>This step failed.</strong></p>");
      return `<section>${parts.join("\n")}</section>`;
    })
    .join("\n");
  const taken = e.redactions.length
    ? `<p><em>Taken out as secrets: ${escapeHtml(e.redactions.join(", "))}.</em></p>`
    : "";
  return `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>${escapeHtml(e.title)}</title>
<style>
body { font-family: "Segoe UI", system-ui, sans-serif; max-width: 60rem; margin: 2rem auto; padding: 0 1rem; line-height: 1.5; color: #1b1b1b; }
pre { background: #0c0c0c; color: #cccccc; padding: 0.75rem 1rem; overflow-x: auto; border-radius: 6px; }
pre.cmd { background: #1e1e1e; color: #f3f3f3; }
blockquote { margin: 0; padding-left: 1rem; border-left: 3px solid #bbb; color: #555; }
img { max-width: 100%; border-radius: 6px; }
section { margin-bottom: 2.5rem; }
</style>
</head>
<body>
<h1>${escapeHtml(e.title)}</h1>
${steps}
${taken}
</body>
</html>
`;
}

/** How a recording is drawn: the operator's font and colours. */
export interface Look {
  theme: ITheme;
  fontFamily: string;
  fontSizePx: number;
  /** The part of the screen to show; the recording's size if not given. */
  area?: Area;
}

function canvasFor(
  rec: recording.Recording,
  look: Look,
): { canvas: HTMLCanvasElement; ctx: CanvasRenderingContext2D; m: Metrics } {
  const canvas = document.createElement("canvas");
  const ctx = canvas.getContext("2d", { willReadFrequently: true });
  if (!ctx) throw new Error("this window cannot draw on a canvas");
  const m = measure(ctx, look.fontFamily, look.fontSizePx);
  Object.assign(canvas, canvasSize(areaOf(rec, look), m));
  return { canvas, ctx, m };
}

const areaOf = (rec: recording.Recording, look: Look): Area =>
  look.area ?? { cols: rec.width, rows: rec.height };

/** Every step's last frame, as a PNG, keyed by step. */
export async function stepStills(e: recording.Export, look: Look): Promise<Map<string, Blob>> {
  const { canvas, ctx, m } = canvasFor(e.recording, look);
  const player = new Player(e.recording);
  const ends = new Map<string, number>();
  for (const ev of e.recording.events) {
    const end = ev.kind === "marker" ? /^end:([^:]+):/.exec(ev.data) : null;
    if (end) ends.set(end[1], ev.at);
  }
  const stills = new Map<string, Blob>();
  for (const s of e.steps) {
    await player.advanceTo(ends.get(s.step) ?? player.duration);
    paint(ctx, screenRows(player.term, look.theme, look.area), m, look.theme);
    const blob = await new Promise<Blob | null>((done) => canvas.toBlob(done, "image/png"));
    if (blob) stills.set(s.step, blob);
  }
  player.dispose();
  return stills;
}

/** The recording as a GIF, at `fps` frames a second, still frames merged. */
export async function toGif(rec: recording.Recording, look: Look, fps = 10): Promise<Uint8Array> {
  const { canvas, ctx, m } = canvasFor(rec, look);
  const player = new Player(rec);
  const gif = GIFEncoder();
  const times = frameTimes(player.duration, fps);
  let previous: Uint8ClampedArray | null = null;
  let held = 0;
  let pending: { index: Uint8Array; palette: number[][] } | null = null;
  const flush = (delay: number) => {
    if (pending) gif.writeFrame(pending.index, canvas.width, canvas.height, { ...pending, delay });
  };
  for (let i = 0; i < times.length; i++) {
    await player.advanceTo(times[i]);
    paint(ctx, screenRows(player.term, look.theme, look.area), m, look.theme);
    const data = ctx.getImageData(0, 0, canvas.width, canvas.height).data;
    const step = i + 1 < times.length ? times[i + 1] - times[i] : 1;
    if (previous && data.length === previous.length && data.every((v, n) => v === previous![n])) {
      held += step;
      continue;
    }
    flush(held * 1000);
    const palette = quantize(data, 64, { format: "rgb444" });
    pending = { index: applyPalette(data, palette, "rgb444"), palette };
    previous = data;
    held = step;
  }
  flush(Math.max(held, 1) * 1000);
  gif.finish();
  player.dispose();
  return gif.bytes();
}

/**
 * The recording as a soundless video. It plays in real time as it is
 * recorded, pauses already shortened; `progress` hears how far it has got.
 */
export async function toVideo(
  rec: recording.Recording,
  look: Look,
  progress: (fraction: number) => void,
): Promise<{ bytes: Uint8Array; extension: string }> {
  const format = videoFormat((m) => MediaRecorder.isTypeSupported(m));
  if (!format) throw new Error("this window cannot record video; export a GIF instead");
  const { canvas, ctx, m } = canvasFor(rec, look);
  const player = new Player(rec);
  const stream = canvas.captureStream(30);
  const recorder = new MediaRecorder(stream, {
    mimeType: format.mime,
    videoBitsPerSecond: 4_000_000,
  });
  const chunks: Blob[] = [];
  recorder.ondataavailable = (ev) => ev.data.size && chunks.push(ev.data);
  const stopped = new Promise<void>((done) => (recorder.onstop = () => done()));
  paint(ctx, screenRows(player.term, look.theme, look.area), m, look.theme);
  recorder.start(1000);
  const start = performance.now();
  const duration = player.duration + 1;
  for (;;) {
    const at = (performance.now() - start) / 1000;
    await player.advanceTo(at);
    paint(ctx, screenRows(player.term, look.theme, look.area), m, look.theme);
    progress(Math.min(1, at / duration));
    if (at >= duration) break;
    await new Promise((done) => setTimeout(done, 33));
  }
  recorder.stop();
  await stopped;
  player.dispose();
  const blob = new Blob(chunks, { type: format.mime });
  return { bytes: new Uint8Array(await blob.arrayBuffer()), extension: format.extension };
}

/** The first `seconds` of a recording, drawn on `canvas` as a preview. */
export async function preview(
  rec: recording.Recording,
  look: Look,
  canvas: HTMLCanvasElement,
  seconds: number,
  cancelled: () => boolean,
): Promise<void> {
  const ctx = canvas.getContext("2d");
  if (!ctx) return;
  const m = measure(ctx, look.fontFamily, look.fontSizePx);
  Object.assign(canvas, canvasSize(areaOf(rec, look), m));
  const player = new Player(rec);
  const start = performance.now();
  const end = Math.min(seconds, player.duration);
  for (;;) {
    if (cancelled()) break;
    const at = (performance.now() - start) / 1000;
    await player.advanceTo(at);
    paint(ctx, screenRows(player.term, look.theme, look.area), m, look.theme);
    if (at >= end) break;
    await new Promise((done) => setTimeout(done, 50));
  }
  player.dispose();
}
