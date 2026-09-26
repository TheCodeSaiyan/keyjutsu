import { forwardRef, useEffect, useImperativeHandle, useRef, useState } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";
import type { KeyChord, TerminalProfile, TerminalSize } from "@keyjutsu/types";
import { chordFromKeyboardEvent } from "../keys";
import { optionsFromProfile, paddingFromProfile } from "../theme";
import { fits, type StagedLine } from "../staging";

export interface TerminalHandle {
  write(data: string): void;
  reset(): void;
  focus(): void;
  size(): TerminalSize;
  /** The text of the cursor's line, up to the cursor: the shell's prompt, between commands. */
  promptText(): string;
  /**
   * Show `lines` on the rows below the cursor, on a layer over the terminal
   * (a staged error). Returns false, showing nothing, if they would not fit.
   */
  stage(lines: StagedLine[]): boolean;
  /** Take the staged lines away. */
  unstage(): void;
}

interface Props {
  profile: TerminalProfile | null;
  /** While true, keys are described to Rust instead of typed by xterm. */
  armed: boolean;
  onData(data: string): void;
  onKey(chord: KeyChord): void;
  onResize(size: TerminalSize): void;
  onTitle(title: string): void;
}

/**
 * The real terminal: xterm.js drawing exactly what the pseudo-console sends.
 * While armed it does not type anything itself; the key is passed on and the
 * shell's own echo is what appears. It never writes anything the shell did
 * not send into the terminal. A staged error, when the operator chose one, is
 * a layer laid over the rows below the prompt while KeyJutsu waits, and is
 * taken away when they answer.
 */
export const TerminalView = forwardRef<TerminalHandle, Props>(function TerminalView(props, ref) {
  const host = useRef<HTMLDivElement>(null);
  const term = useRef<Terminal | null>(null);
  const fit = useRef<FitAddon | null>(null);
  // Handlers read the latest props without recreating the terminal.
  const latest = useRef(props);
  useEffect(() => {
    latest.current = props;
  });

  const frame = useRef<HTMLDivElement>(null);
  // A staged error: its lines, the row they start on, and where that row is
  // drawn, worked out from xterm's own rows so the layer lines up with them.
  const [layer, setLayer] = useState<{
    lines: StagedLine[];
    row: number;
    top: number;
    left: number;
    width: number;
    rowHeight: number;
  } | null>(null);
  const layerRef = useRef(layer);
  layerRef.current = layer;
  const savedTheme = useRef<Terminal["options"]["theme"] | null>(null);

  /** Where row `row` of the screen is drawn, relative to the frame. */
  const rowBox = (row: number) => {
    const rows = term.current?.element?.querySelector(".xterm-rows");
    const target = rows?.children[row] as HTMLElement | undefined;
    const outer = frame.current?.getBoundingClientRect();
    if (!rows || !target || !outer) return null;
    const r = target.getBoundingClientRect();
    const all = rows.getBoundingClientRect();
    return {
      top: r.top - outer.top,
      left: all.left - outer.left,
      width: all.width,
      rowHeight: r.height,
    };
  };

  useImperativeHandle(ref, () => ({
    write: (data) => term.current?.write(data),
    reset: () => {
      setLayer(null);
      term.current?.reset();
    },
    focus: () => term.current?.focus(),
    size: () => ({ rows: term.current?.rows ?? 30, cols: term.current?.cols ?? 120 }),
    promptText: () => {
      const b = term.current?.buffer.active;
      const line = b?.getLine(b.baseY + b.cursorY);
      return b && line ? line.translateToString(false, 0, b.cursorX) : "";
    },
    stage: (lines) => {
      const t = term.current;
      if (!t || layerRef.current) return false;
      const b = t.buffer.active;
      if (!fits(b.cursorY, t.rows, lines.length)) return false;
      const box = rowBox(b.cursorY + 1);
      if (!box) return false;
      // The real cursor stays on the real prompt, above the staged one: it
      // is hidden while the staged prompt shows its own.
      savedTheme.current = t.options.theme ?? {};
      t.options.theme = { ...t.options.theme, cursor: "#00000000", cursorAccent: "#00000000" };
      setLayer({ lines, row: b.cursorY + 1, ...box });
      return true;
    },
    unstage: () => {
      const t = term.current;
      if (t && savedTheme.current) t.options.theme = savedTheme.current;
      savedTheme.current = null;
      setLayer(null);
    },
  }));

  useEffect(() => {
    const t = new Terminal({ allowProposedApi: false, scrollback: 5000, fontSize: 16 });
    const f = new FitAddon();
    t.loadAddon(f);
    t.open(host.current!);
    f.fit();
    term.current = t;
    fit.current = f;

    t.attachCustomKeyEventHandler((ev) => {
      if (!latest.current.armed) return true;
      if (ev.type === "keydown") {
        const chord = chordFromKeyboardEvent(ev);
        if (chord) latest.current.onKey(chord);
      }
      // Stop xterm and the browser producing any input from this key.
      ev.preventDefault();
      return false;
    });
    // Typed data while unarmed, plus replies xterm sends on its own (cursor
    // position, focus). Rust accepts only the replies while armed.
    const data = t.onData((d) => latest.current.onData(d));
    const resize = t.onResize(({ rows, cols }) => latest.current.onResize({ rows, cols }));
    const title = t.onTitleChange((s) => latest.current.onTitle(s));

    const observer = new ResizeObserver(() => {
      fit.current?.fit();
      // A staged layer follows its rows when the terminal is resized.
      requestAnimationFrame(() => {
        const l = layerRef.current;
        const box = l && rowBox(l.row);
        if (l && box) setLayer({ ...l, ...box });
      });
    });
    observer.observe(host.current!);
    return () => {
      observer.disconnect();
      data.dispose();
      resize.dispose();
      title.dispose();
      t.dispose();
      term.current = null;
    };
  }, []);

  useEffect(() => {
    const t = term.current;
    if (!t || !props.profile) return;
    const options = optionsFromProfile(props.profile);
    t.options.fontFamily = options.fontFamily;
    t.options.fontSize = options.fontSize;
    t.options.cursorStyle = options.cursorStyle;
    t.options.cursorBlink = options.cursorBlink;
    t.options.theme = options.theme;
    fit.current?.fit();
  }, [props.profile]);

  useEffect(() => {
    if (props.armed) term.current?.focus();
    // Chrome appearing or disappearing changes the space available.
    requestAnimationFrame(() => fit.current?.fit());
  }, [props.armed]);

  return (
    <div
      ref={frame}
      className="terminal-frame"
      style={{
        padding: props.profile ? paddingFromProfile(props.profile) : "8px",
        background: props.profile?.color_scheme.background ?? "#0C0C0C",
      }}
    >
      <div ref={host} className="terminal-host" role="application" aria-label="Terminal" />
      {layer && (
        <div
          className="staged-layer"
          aria-hidden="true"
          style={{
            top: layer.top,
            left: layer.left,
            width: layer.width,
            fontFamily: term.current?.options.fontFamily,
            fontSize: term.current?.options.fontSize,
            lineHeight: `${layer.rowHeight}px`,
            background: savedTheme.current?.background ?? props.profile?.color_scheme.background,
            color: savedTheme.current?.foreground,
          }}
        >
          {layer.lines.map((l, i) => (
            <div
              key={i}
              style={{
                height: layer.rowHeight,
                color: l.error ? (savedTheme.current?.brightRed ?? "#E74856") : undefined,
              }}
            >
              {l.text}
              {i === layer.lines.length - 1 && (
                <span
                  className="staged-cursor"
                  data-blink={term.current?.options.cursorBlink ? "true" : undefined}
                  data-style={term.current?.options.cursorStyle}
                  style={{
                    background: savedTheme.current?.cursor ?? savedTheme.current?.foreground,
                  }}
                />
              )}
            </div>
          ))}
        </div>
      )}
    </div>
  );
});
