import type { agent } from "@keyjutsu/types";

/**
 * A reply to the agent, waiting its turn. The agent works on one request at a
 * time, so what the operator says meanwhile is queued, and sent when it is
 * free, rather than refused.
 */
export interface Reply {
  id: number;
  /** The agent it was written to, chosen when it was written. */
  agent: agent.AgentKind;
  /** The step it is about, or `null` for the whole plan. */
  step: string | null;
  /** The step's title when it was written, for when the step has gone by the time it is sent. */
  stepTitle: string | null;
  text: string;
}

/** One request to the agent, made of one or more replies. */
export interface Batch {
  agent: agent.AgentKind;
  step: string | null;
  guidance: string;
  replies: Reply[];
}

/**
 * What to send next: the oldest reply, with the replies straight after it to
 * the same agent about the same thing, as one request, so three quick replies
 * cost one revision and not three. Order is kept: a reply about something else
 * waits for its own turn. A reply about a step that has since gone goes to the
 * plan, saying which step it meant.
 */
export function nextBatch(queue: Reply[], steps: string[]): Batch | null {
  const first = queue[0];
  if (!first) return null;
  const gone = (r: Reply) => r.step !== null && !steps.includes(r.step);
  const target = (r: Reply) => (gone(r) ? null : r.step);
  const to = target(first);
  const replies: Reply[] = [];
  for (const r of queue) {
    if (r.agent !== first.agent || target(r) !== to) break;
    replies.push(r);
  }
  const guidance = replies
    .map((r) =>
      gone(r) ? `About the step "${r.stepTitle ?? r.step}", since removed: ${r.text}` : r.text,
    )
    .join("\n\n");
  return { agent: first.agent, step: to, guidance, replies };
}

/** The queue as the Plan screen uses it. */
export interface Replies {
  queued: Reply[];
  /** The replies the agent is working on now. */
  sending: number[];
  /** A send failed: the rest wait until the operator sends again. */
  held: boolean;
  add(reply: Omit<Reply, "id">): void;
  remove(id: number): void;
  resume(): void;
}
