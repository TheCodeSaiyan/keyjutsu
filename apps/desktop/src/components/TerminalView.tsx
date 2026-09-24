import { forwardRef, useEffect, useImperativeHandle, useRef } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";
import type { KeyChord, TerminalProfile, TerminalSize } from "@keyjutsu/types";
import { chordFromKeyboardEvent } from "../keys";
import { optionsFromProfile, paddingFromProfile } from "../theme";

export interface TerminalHandle {
  write(data: string): void;
  reset(): void;
  focus(): void;
  size(): TerminalSize;
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
 * It never fabricates output, and while armed it does not type anything
 * itself; the key is passed on and the shell's own echo is what appears.
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

  useImperativeHandle(ref, () => ({
    write: (data) => term.current?.write(data),
    reset: () => term.current?.reset(),
    focus: () => term.current?.focus(),
    size: () => ({ rows: term.current?.rows ?? 30, cols: term.current?.cols ?? 120 }),
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

    const observer = new ResizeObserver(() => fit.current?.fit());
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
      className="terminal-frame"
      style={{
        padding: props.profile ? paddingFromProfile(props.profile) : "8px",
        background: props.profile?.color_scheme.background ?? "#0C0C0C",
      }}
    >
      <div ref={host} className="terminal-host" role="application" aria-label="Terminal" />
    </div>
  );
});
