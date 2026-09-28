import type { execute, history, plan } from "@keyjutsu/types";

/**
 * How a past run is shown: its outcome as a badge, its times in the
 * operator's own clock, its steps by title, and why it stopped.
 */

export type Tone = "ready" | "review" | "blocked" | "none";

/** The history list's outcome words, as a badge. */
export function outcomeBadge(outcome: string): { label: string; tone: Tone } {
  if (outcome === "complete") return { label: "Completed", tone: "ready" };
  if (outcome === "disarmed") return { label: "Disarmed", tone: "none" };
  if (outcome === "blocked") return { label: "Not started", tone: "blocked" };
  if (outcome.startsWith("failed")) return { label: "Failed", tone: "blocked" };
  if (outcome.startsWith("waiting")) return { label: "Waiting", tone: "review" };
  return { label: outcome, tone: "none" };
}

/** `2026-09-28T16:46:06Z` as the operator's clock shows it: `Mon 28 Sept, 17:46`. */
export function when(iso: string, locale?: string, timeZone?: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  const day = d.toLocaleDateString(locale ?? "en-GB", {
    weekday: "short",
    day: "numeric",
    month: "short",
    timeZone,
  });
  const time = d.toLocaleTimeString(locale ?? "en-GB", {
    hour: "2-digit",
    minute: "2-digit",
    timeZone,
  });
  return `${day.replace(",", "")}, ${time}`;
}

/** How long from `start` to `end`: `under a second`, `45 s`, `2 min 14 s`, `1 h 3 min`. */
export function duration(start: string, end: string): string {
  const s = Math.max(0, Math.round((new Date(end).getTime() - new Date(start).getTime()) / 1000));
  if (!Number.isFinite(s)) return "";
  if (s < 1) return "under a second";
  if (s < 60) return `${s} s`;
  if (s < 3600) return s % 60 ? `${Math.floor(s / 60)} min ${s % 60} s` : `${s / 60} min`;
  const m = Math.floor(s / 60);
  return m % 60 ? `${Math.floor(m / 60)} h ${m % 60} min` : `${m / 60} h`;
}

/** Each step's title, from the run's sealed snapshot. */
export function stepTitles(snapshot: string): Map<string, string> {
  try {
    const parsed = JSON.parse(snapshot) as { plan?: plan.Plan };
    return new Map((parsed.plan?.steps ?? []).map((s) => [s.id, s.title]));
  } catch {
    return new Map();
  }
}

/** Why a run did not complete, in words, or null if it did. */
export function stoppedBecause(
  outcome: execute.Outcome,
  titles: Map<string, string>,
): string | null {
  const name = (step: string | null) => (step ? `“${titles.get(step) ?? step}”` : "a step");
  switch (outcome.kind) {
    case "complete":
      return null;
    case "failed":
      return `${name(outcome.step)} failed: expected ${outcome.expected}, got ${outcome.actual}.`;
    case "aborted":
      return outcome.in_doubt
        ? `Disarmed during ${name(outcome.step)} after its command was sent, so its effect is unknown.`
        : "Disarmed. Nothing further ran.";
    case "blocked":
      return `It did not start: ${outcome.reason}`;
    case "boundary":
      return `Waiting to continue after phase ${outcome.phase}.`;
  }
}

const AGENTS: Record<string, string> = {
  codex: "Codex",
  claude_code: "Claude Code",
  gemini: "Gemini",
  github_copilot: "GitHub Copilot",
  cursor: "Cursor",
};

export const agentLabel = (a: plan.Agent) => `${AGENTS[a.name] ?? a.name} ${a.version}`;

/** Newest first. */
export const newestFirst = (runs: history.SessionSummary[]) =>
  [...runs].sort((a, b) => b.finished_at.localeCompare(a.finished_at));
