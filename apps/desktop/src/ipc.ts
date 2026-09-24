import { Channel, invoke } from "@tauri-apps/api/core";
import type {
  KeyChord,
  OpenRequest,
  PerformanceConfig,
  PerformanceSnapshot,
  ReadinessReport,
  ScriptSource,
  TerminalMessage,
  TerminalProfile,
  TerminalSize,
} from "@keyjutsu/types";

/**
 * Typed wrappers over the Rust commands. Nothing here decides anything: each
 * call is a request that keyjutsu-core may refuse.
 */
export const ipc = {
  readinessScan: () => invoke<ReadinessReport>("readiness_scan"),
  terminalProfile: () => invoke<TerminalProfile>("terminal_profile"),

  openTerminal: (request: OpenRequest, onMessage: (m: TerminalMessage) => void) => {
    const channel = new Channel<TerminalMessage>();
    channel.onmessage = onMessage;
    return invoke<number>("terminal_open", { request, onMessage: channel });
  },
  write: (id: number, data: string) => invoke<void>("terminal_write", { id, data }),
  key: (id: number, chord: KeyChord) => invoke<void>("terminal_key", { id, chord }),
  resize: (id: number, size: TerminalSize) => invoke<void>("terminal_resize", { id, size }),
  close: (id: number) => invoke<void>("terminal_close", { id }),

  arm: (id: number, source: ScriptSource, config: PerformanceConfig) =>
    invoke<PerformanceSnapshot>("performance_arm", { id, source, config }),
  disarm: (id: number) => invoke<void>("performance_disarm", { id }),
  pause: (id: number) => invoke<void>("performance_pause", { id }),
  resume: (id: number) => invoke<void>("performance_resume", { id }),
};

/** The refusal Rust returns for typed input while a performance owns the keyboard. */
export const INPUT_OWNED = "input_owned";
