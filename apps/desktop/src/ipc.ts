import { Channel, invoke } from "@tauri-apps/api/core";
import type {
  agent,
  history,
  plan,
  recovery,
  technique,
  workspace,
  KeyChord,
  OpenRequest,
  PerformanceConfig,
  PerformanceSnapshot,
  ReadinessReport,
  RunMessage,
  ScriptSource,
  Sealed,
  TerminalMessage,
  TerminalProfile,
  TerminalSize,
  TechniqueDraft,
  WaitingOpened,
  WaitingRun,
} from "@keyjutsu/types";

/**
 * Typed wrappers over the Rust commands. Nothing here decides anything: each
 * call is a request that keyjutsu-core may refuse.
 */
export const ipc = {
  readinessScan: () => invoke<ReadinessReport>("readiness_scan"),
  /** Open one of KeyJutsu's own download links; Rust refuses any other. */
  openLink: (url: string) => invoke<void>("open_link", { url }),
  /** The diagnostic bundle, in full, as it would be saved. */
  diagnosticsPreview: () => invoke<string>("diagnostics_preview"),
  /** Save the bundle last previewed; returns where. */
  diagnosticsSave: () => invoke<string>("diagnostics_save"),
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

  agents: () => invoke<agent.AgentInfo[]>("agents_list"),
  workspace: () => invoke<workspace.WorkspaceView | null>("workspace_view"),
  openPlan: (text: string) => invoke<workspace.WorkspaceView>("workspace_open", { text }),
  propose: (task: string, agent: agent.AgentKind, context: string) =>
    invoke<workspace.WorkspaceView>("workspace_propose", { task, agent, context }),
  replaceStep: (step: plan.Step) =>
    invoke<workspace.WorkspaceView>("workspace_replace_step", { step }),
  insertStep: (after: string | null, step: plan.Step) =>
    invoke<workspace.WorkspaceView>("workspace_insert_step", { after, step }),
  removeStep: (id: string) => invoke<workspace.WorkspaceView>("workspace_remove_step", { id }),
  moveStep: (id: string, earlier: boolean) =>
    invoke<workspace.WorkspaceView>("workspace_move_step", { id, earlier }),
  note: (text: string, step: string | null) =>
    invoke<workspace.WorkspaceView>("workspace_note", { text, step }),
  validate: () => invoke<workspace.WorkspaceView>("workspace_validate"),
  /** Download and pin the plan's artifacts now, so nothing is fetched while it runs. */
  stage: () => invoke<workspace.WorkspaceView>("workspace_stage"),
  retryStep: (agent: agent.AgentKind, step: string, guidance: string) =>
    invoke<workspace.WorkspaceView>("workspace_retry_step", { agent, step, guidance }),
  /** Recorded runs, newest first. */
  history: () => invoke<history.SessionSummary[]>("history_list"),
  historyShow: (id: string) => invoke<history.SessionRecord>("history_show", { id }),
  /** Make a completed run a Technique; each pair is a parameter and the value it stands for. */
  promote: (session: string, name: string, description: string, params: [string, string][]) =>
    invoke<technique.Technique>("technique_promote", { session, name, description, params }),
  techniques: () => invoke<technique.Technique[]>("technique_list"),
  /** A Technique as a draft plan in the workspace, to validate and approve. */
  useTechnique: (id: string, values: Record<string, string>) =>
    invoke<TechniqueDraft>("technique_use", { id, values }),
  /** Ask the agent to fix the step the last run failed at, from what it printed. */
  fixFailure: (agent: agent.AgentKind, guidance: string) =>
    invoke<workspace.WorkspaceView>("workspace_fix_failure", { agent, guidance }),
  revise: (agent: agent.AgentKind, guidance: string) =>
    invoke<workspace.WorkspaceView>("workspace_revise", { agent, guidance }),
  review: (agent: agent.AgentKind) =>
    invoke<workspace.WorkspaceView>("workspace_review", { agent }),
  approve: (confirmations: Record<string, string>) =>
    invoke<Sealed>("workspace_approve", { confirmations }),

  runPlan: (id: number, config: PerformanceConfig, onEvent: (m: RunMessage) => void) => {
    const channel = new Channel<RunMessage>();
    channel.onmessage = onEvent;
    return invoke<void>("plan_run", { id, config, onEvent: channel });
  },
  /** What the operator typed for a critical step, or null to decline. Rust compares it. */
  confirm: (typed: string | null) => invoke<void>("plan_confirm", { typed }),
  /** A run stopped at a restart or other boundary, waiting to continue. */
  waitingRun: () => invoke<WaitingRun | null>("run_waiting"),
  /** Open it to continue: the next run resumes, and asks before crossing. */
  openWaiting: () => invoke<WaitingOpened>("run_waiting_open"),
  recoveryPlan: () => invoke<recovery.RecoveryItem[]>("recovery_plan"),
  recover: (id: number) => invoke<recovery.RecoveryResult[]>("recovery_run", { id }),
};

/** The refusal Rust returns for typed input while a performance owns the keyboard. */
export const INPUT_OWNED = "input_owned";
