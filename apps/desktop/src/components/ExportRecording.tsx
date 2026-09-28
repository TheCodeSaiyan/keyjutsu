import { useEffect, useRef, useState } from "react";
import type { recording, TerminalProfile } from "@keyjutsu/types";
import { ipc } from "../ipc";
import { themeFromProfile } from "../theme";
import { areas, type Area } from "../recording/render";
import {
  guideHtml,
  guideMarkdown,
  preview,
  stepFileBase,
  stepStills,
  toGif,
  toVideo,
  type Look,
} from "../recording/export";

interface Props {
  /** The history record of a run that was recorded. */
  session: string;
  profile: TerminalProfile | null;
  onClose(): void;
}

type Which = "all" | "range" | "each";

const encoder = new TextEncoder();

async function dataUri(blob: Blob): Promise<string> {
  return new Promise((done) => {
    const reader = new FileReader();
    reader.onload = () => done(String(reader.result));
    reader.readAsDataURL(blob);
  });
}

/**
 * Export a recorded run (ADR 0021): the whole run, a range of steps, or each
 * step on its own, as a soundless video, a GIF, an asciicast and a guide.
 * Rust has already cut it, taken out what looked like secrets and shortened
 * long pauses; the first seconds play here before anything is saved.
 */
export function ExportRecording({ session, profile, onClose }: Props) {
  const [whole, setWhole] = useState<recording.Export | null>(null);
  const [which, setWhich] = useState<Which>("all");
  const [from, setFrom] = useState("");
  const [to, setTo] = useState("");
  const [formats, setFormats] = useState({ video: true, gif: false, cast: false, guide: true });
  const [working, setWorking] = useState<string | null>(null);
  const [saved, setSaved] = useState<string[]>([]);
  const [error, setError] = useState<string | null>(null);
  const canvas = useRef<HTMLCanvasElement>(null);
  // How much of the screen the run drew on, and the whole of it.
  const [found, setFound] = useState<{ used: Area; whole: Area } | null>(null);
  const [size, setSize] = useState<"fit" | "whole">("fit");
  const [fontPx, setFontPx] = useState(14);

  const look: Look = {
    theme: profile ? themeFromProfile(profile) : { background: "#0c0c0c", foreground: "#cccccc" },
    fontFamily: profile
      ? `"${profile.font_face}", "Cascadia Mono", Consolas, monospace`
      : "Consolas, monospace",
    fontSizePx: fontPx,
    area: found ? (size === "fit" ? found.used : found.whole) : undefined,
  };

  useEffect(() => {
    ipc
      .recordingExport(session, null, null)
      .then(async (e) => {
        setFound(await areas(e.recording));
        setWhole(e);
        setFrom(e.steps[0]?.step ?? "");
        setTo(e.steps[e.steps.length - 1]?.step ?? "");
      })
      .catch((e) => setError(String(e)));
  }, [session]);

  // The preview: the first seconds of the chosen part.
  useEffect(() => {
    if (!whole || !canvas.current) return;
    let stop = false;
    const first = which === "range" ? from : null;
    const last = which === "range" ? to : null;
    const load =
      which === "all" ? Promise.resolve(whole) : ipc.recordingExport(session, first, last);
    void load
      .then((e) => preview(e.recording, look, canvas.current!, 6, () => stop))
      .catch((e) => setError(String(e)));
    return () => {
      stop = true;
    };
    // The look follows the profile, which does not change while this is open.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [whole, which, from, to, session, size, fontPx, found]);

  const steps = whole?.steps ?? [];
  const folder = `${session}${which === "range" ? `-${from}-to-${to}` : which === "each" ? "-steps" : ""}`;

  const exportOne = async (e: recording.Export, base: string, out: string[]) => {
    const save = async (name: string, bytes: Uint8Array) => {
      out.push(await ipc.recordingSave(folder, name, bytes));
    };
    if (formats.cast) await save(`${base}.cast`, encoder.encode(e.cast));
    if (formats.gif) {
      setWorking(`Drawing the GIF for ${e.title}…`);
      await save(`${base}.gif`, await toGif(e.recording, look));
    }
    if (formats.video) {
      const v = await toVideo(e.recording, look, (f) =>
        setWorking(`Recording the video for ${e.title}: ${Math.round(f * 100)}%`),
      );
      await save(`${base}.${v.extension}`, v.bytes);
    }
  };

  const run = async () => {
    if (!whole) return;
    setError(null);
    setSaved([]);
    const out: string[] = [];
    try {
      const whole_or_range =
        which === "range" ? await ipc.recordingExport(session, from, to) : whole;
      if (which === "each") {
        for (const [i, s] of steps.entries()) {
          setWorking(`Step ${i + 1} of ${steps.length}: ${s.title}…`);
          await exportOne(
            await ipc.recordingExport(session, s.step, s.step),
            stepFileBase(i + 1, s.step),
            out,
          );
        }
      } else {
        await exportOne(whole_or_range, "recording", out);
      }
      if (formats.guide) {
        setWorking("Writing the guide…");
        const stills = await stepStills(whole_or_range, look);
        const files = new Map<string, string>();
        const inline = new Map<string, string>();
        for (const [i, s] of whole_or_range.steps.entries()) {
          const png = stills.get(s.step);
          if (!png) continue;
          const name = `${stepFileBase(i + 1, s.step)}.png`;
          out.push(await ipc.recordingSave(folder, name, new Uint8Array(await png.arrayBuffer())));
          files.set(s.step, name);
          inline.set(s.step, await dataUri(png));
        }
        out.push(
          await ipc.recordingSave(
            folder,
            "guide.md",
            encoder.encode(guideMarkdown(whole_or_range, files)),
          ),
        );
        out.push(
          await ipc.recordingSave(
            folder,
            "guide.html",
            encoder.encode(guideHtml(whole_or_range, inline)),
          ),
        );
      }
      setSaved(out);
      await ipc.recordingReveal(folder);
    } catch (e) {
      setError(String(e));
    } finally {
      setWorking(null);
    }
  };

  const nothingChosen = !formats.video && !formats.gif && !formats.cast && !formats.guide;

  return (
    <div className="scrim" role="dialog" aria-label="Export the recording">
      <div className="card stack export-recording">
        <h2>Export the recording</h2>
        {!whole && !error && <p className="muted">Loading the recording…</p>}
        {whole && (
          <>
            <canvas
              ref={canvas}
              className="recording-preview"
              aria-label="Preview of the first seconds"
            />
            <p className="small">
              {whole.redactions.length
                ? `Taken out as secrets: ${whole.redactions.join(", ")}. Each is shown as *.`
                : "Nothing in it looked like a secret. Check the preview all the same."}
            </p>
            <fieldset>
              <legend>What to export</legend>
              <label className="choice">
                <input
                  type="radio"
                  name="which"
                  checked={which === "all"}
                  onChange={() => setWhich("all")}
                />
                The whole run ({steps.length} {steps.length === 1 ? "step" : "steps"})
              </label>
              <label className="choice">
                <input
                  type="radio"
                  name="which"
                  checked={which === "range"}
                  onChange={() => setWhich("range")}
                />
                From{" "}
                <select
                  value={from}
                  onChange={(e) => setFrom(e.target.value)}
                  disabled={which !== "range"}
                >
                  {steps.map((s, i) => (
                    <option key={s.step} value={s.step}>
                      {i + 1}. {s.title}
                    </option>
                  ))}
                </select>{" "}
                to{" "}
                <select
                  value={to}
                  onChange={(e) => setTo(e.target.value)}
                  disabled={which !== "range"}
                >
                  {steps.map((s, i) => (
                    <option key={s.step} value={s.step}>
                      {i + 1}. {s.title}
                    </option>
                  ))}
                </select>
              </label>
              <label className="choice">
                <input
                  type="radio"
                  name="which"
                  checked={which === "each"}
                  onChange={() => setWhich("each")}
                />
                Each step as its own file
              </label>
            </fieldset>
            <fieldset>
              <legend>Size</legend>
              <label className="choice">
                <input
                  type="radio"
                  name="size"
                  checked={size === "fit"}
                  onChange={() => setSize("fit")}
                />
                Fit to what was drawn{found ? ` (${found.used.cols} × ${found.used.rows})` : ""}
              </label>
              <label className="choice">
                <input
                  type="radio"
                  name="size"
                  checked={size === "whole"}
                  onChange={() => setSize("whole")}
                />
                As recorded{found ? ` (${found.whole.cols} × ${found.whole.rows})` : ""}
              </label>
              <label>
                Text size{" "}
                <select value={fontPx} onChange={(e) => setFontPx(Number(e.target.value))}>
                  {[12, 14, 16, 18].map((px) => (
                    <option key={px} value={px}>
                      {px} px
                    </option>
                  ))}
                </select>
              </label>
            </fieldset>
            <fieldset>
              <legend>As</legend>
              {(
                [
                  ["video", "Video, no sound (MP4, or WebM where MP4 cannot be made)"],
                  ["gif", "GIF"],
                  ["cast", "Asciicast (.cast), for asciinema"],
                  ["guide", "Step-by-step guide: Markdown with pictures, and one HTML page"],
                ] as const
              ).map(([key, label]) => (
                <label key={key} className="choice">
                  <input
                    type="checkbox"
                    checked={formats[key]}
                    onChange={(e) => setFormats({ ...formats, [key]: e.target.checked })}
                  />
                  {label}
                </label>
              ))}
            </fieldset>
            <p className="small muted">
              A video plays through as it is made, with long pauses cut to two seconds. Keep this
              window in view while it exports: Windows slows a window that is hidden or minimised.
              Files go to Videos\KeyJutsu\{folder}.
            </p>
          </>
        )}
        {working && (
          <p role="status" className="small">
            {working}
          </p>
        )}
        {saved.length > 0 && <p className="small">Saved {saved.length} files.</p>}
        {error && (
          <p role="alert" className="error">
            {error}
          </p>
        )}
        <div className="row">
          <button
            className="primary"
            disabled={!whole || working !== null || nothingChosen}
            onClick={() => void run()}
          >
            Export
          </button>
          <button onClick={onClose} disabled={working !== null}>
            Close
          </button>
        </div>
      </div>
    </div>
  );
}
