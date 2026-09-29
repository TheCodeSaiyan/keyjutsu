import { useState } from "react";
import type { agent, plan, workspace } from "@keyjutsu/types";
import { ipc } from "../ipc";
import type { Replies } from "../replies";

/**
 * What needs the operator in a step, or in the whole plan, and the choices
 * that answer it (ADR 0020). Rust writes the asks and their choices, from
 * the kind of finding, never from what an agent wrote; each choice here only
 * calls the workspace operation it names. Free text is always possible: for
 * a step it goes to the agent as that step's guidance, for the plan as the
 * plan's; or it is kept as the operator's note. A reply written while the
 * agent is working is queued, and sent when it is free.
 */
export function Conversation({
  asks,
  step,
  primary,
  busy,
  replies,
  act,
  onEdit,
}: {
  asks: workspace.Ask[];
  /** The step this conversation is about; `undefined` for the whole plan. */
  step: plan.Step | undefined;
  primary: agent.AgentKind | undefined;
  busy: string | null;
  replies: Replies;
  act(label: string, request: () => Promise<workspace.WorkspaceView>): void;
  onEdit?(): void;
}) {
  const [text, setText] = useState("");
  const [dismissing, setDismissing] = useState<number | null>(null);
  const [reason, setReason] = useState("");
  const [answering, setAnswering] = useState<string | null>(null);
  const [answer, setAnswer] = useState("");

  const sendAnswer = (question: string, text: string) => {
    if (!primary) return;
    act("The agent is reconsidering the plan with your answer…", () =>
      ipc.answer(primary, question, text),
    );
  };

  const askAgent = (guidance: string) => {
    if (!primary) return;
    if (step) {
      act("The agent is revising this step…", () => ipc.retryStep(primary, step.id, guidance));
    } else {
      act("The agent is reconsidering the plan…", () => ipc.revise(primary, guidance));
    }
  };

  const choose = (c: workspace.Choice) => {
    switch (c.kind) {
      case "stage_downloads":
        act("Downloading and checking artifacts…", () => ipc.stage());
        break;
      case "use_key_jutsu_rating":
        if (step) act("Using KeyJutsu's rating…", () => ipc.useKeyJutsuRating(step.id));
        break;
      case "ask_agent":
        askAgent(c.guidance);
        break;
      case "validate_again":
        act("Validating…", () => ipc.validate());
        break;
      case "edit_step":
        onEdit?.();
        break;
      case "remove_step":
        if (step) act("Removing the step…", () => ipc.removeStep(step.id));
        break;
      case "dismiss":
        // Fine needs no reason: one click. "Because…" is its own button.
        act("Noting that…", () => ipc.dismiss(c.note, ""));
        break;
      case "keep_as_planned":
        act("Noting that…", () => ipc.keepAsPlanned(c.question));
        break;
      case "accept_as_is":
        if (step) act("Accepting the step as it is…", () => ipc.acceptAsIs(step.id));
        break;
      case "answer":
        sendAnswer(c.question, c.answer);
        break;
      case "answer_in_own_words":
        setAnswer("");
        setAnswering(c.question);
        break;
      case "carry_on":
        act("Noting that…", () => ipc.carryOn(c.question));
        break;
    }
  };

  const label = (c: workspace.Choice): string => {
    switch (c.kind) {
      case "stage_downloads":
        return "Stage the download";
      case "use_key_jutsu_rating":
        return "Use KeyJutsu's rating";
      case "ask_agent":
        return c.label;
      case "validate_again":
        return "I've changed the machine: validate again";
      case "edit_step":
        return "Edit the step";
      case "remove_step":
        return "Remove the step";
      case "dismiss":
        return "It's fine";
      case "answer":
        return c.answer;
      case "answer_in_own_words":
        return "In my own words";
      case "carry_on":
        return "Carry on as planned";
      case "keep_as_planned":
        return /as planned|\(default\)\s*$/i.test(c.answer) ? c.answer : `${c.answer} (as planned)`;
      case "accept_as_is":
        return "Run it as it is";
    }
  };

  const target = step?.id ?? null;
  const queued = replies.queued.filter((r) => r.step === target);
  // Only the reply queues while the agent works: a choice answers the plan
  // as it is now, which the agent is about to change.
  const reply = () => {
    const t = text.trim();
    if (!primary || !t) return;
    replies.add({ agent: primary, step: target, stepTitle: step?.title ?? null, text: t });
    setText("");
  };

  const needsAgent = (c: workspace.Choice) =>
    (c.kind === "ask_agent" || c.kind === "answer" || c.kind === "answer_in_own_words") && !primary;

  return (
    <div className="conversation">
      {asks.length === 0 && <p className="small muted">Nothing here needs you.</p>}
      {asks.length > 0 && (
        <ul className="asks">
          {asks.map((a, i) => (
            <li key={i} className="ask">
              <p className="small">
                <strong>{a.from.kind === "validation" ? "KeyJutsu" : a.from.who}</strong>
                {a.from.kind === "agent" ? " asks" : ""}: {a.text}
              </p>
              <div className="row choices">
                {a.choices.map((c, j) => (
                  <button
                    key={j}
                    disabled={busy !== null || needsAgent(c)}
                    title={needsAgent(c) ? "Open this plan with an agent to ask it" : undefined}
                    onClick={() => choose(c)}
                  >
                    {label(c)}
                  </button>
                ))}
                {a.choices.map((c, j) =>
                  c.kind === "dismiss" ? (
                    <button
                      key={`because-${j}`}
                      disabled={busy !== null}
                      onClick={() => {
                        setReason("");
                        setDismissing(c.note);
                      }}
                    >
                      It&apos;s fine, because…
                    </button>
                  ) : null,
                )}
              </div>
              {a.choices.some((c) => c.kind === "dismiss" && c.note === dismissing) && (
                <form
                  className="row"
                  onSubmit={(e) => {
                    e.preventDefault();
                    const note = dismissing!;
                    setDismissing(null);
                    act("Noting that…", () => ipc.dismiss(note, reason.trim()));
                  }}
                >
                  <input
                    aria-label="Why it needs no change"
                    placeholder="Why it needs no change (kept with the plan)"
                    value={reason}
                    onChange={(e) => setReason(e.target.value)}
                  />
                  <button type="submit" disabled={busy !== null}>
                    Dismiss
                  </button>
                  <button type="button" onClick={() => setDismissing(null)}>
                    Cancel
                  </button>
                </form>
              )}
              {a.choices.some(
                (c) => c.kind === "answer_in_own_words" && c.question === answering,
              ) && (
                <form
                  className="row"
                  onSubmit={(e) => {
                    e.preventDefault();
                    const question = answering!;
                    setAnswering(null);
                    sendAnswer(question, answer.trim());
                  }}
                >
                  <input
                    aria-label="Your answer"
                    placeholder="Your answer, sent to the agent"
                    value={answer}
                    onChange={(e) => setAnswer(e.target.value)}
                  />
                  <button type="submit" disabled={busy !== null || !answer.trim()}>
                    Answer
                  </button>
                  <button type="button" onClick={() => setAnswering(null)}>
                    Cancel
                  </button>
                </form>
              )}
            </li>
          ))}
        </ul>
      )}
      <label className="small">
        {step ? "Reply about this step" : "Reply about the plan"}
        <textarea
          rows={3}
          value={text}
          placeholder="What to change, or a note"
          onChange={(e) => setText(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
              e.preventDefault();
              reply();
            }
          }}
        />
      </label>
      <div className="row">
        <button
          disabled={!primary || !text.trim()}
          title={primary ? "Ctrl+Enter sends it too" : "Open this plan with an agent to ask it"}
          onClick={reply}
        >
          {busy !== null ? "Queue for the agent" : "Send to agent"}
        </button>
        <button
          disabled={busy !== null || !text.trim()}
          onClick={() => {
            const t = text.trim();
            setText("");
            act("Saving note…", () => ipc.note(t, step?.id ?? null));
          }}
        >
          Keep as a note
        </button>
      </div>
      {text.trim() && (
        <p className="small muted">
          {busy !== null
            ? "The agent is busy: this waits its turn, and goes when it is free."
            : "What the agent sends back comes back unvalidated and unapproved, like any change."}
        </p>
      )}
      {queued.length > 0 && (
        <div className="queued">
          <p className="small muted">
            {replies.held
              ? "Held: the last reply could not be sent. It is kept here."
              : "Your replies, in the order the agent gets them:"}
          </p>
          <ol>
            {queued.map((r) => {
              const now = replies.sending.includes(r.id);
              return (
                <li key={r.id} data-sending={now}>
                  <span>{r.text}</span>
                  {now ? (
                    <span className="small muted">with the agent now</span>
                  ) : (
                    <button className="link small" onClick={() => replies.remove(r.id)}>
                      Remove
                    </button>
                  )}
                </li>
              );
            })}
          </ol>
          {replies.held && (
            <button disabled={busy !== null} onClick={replies.resume}>
              Send again
            </button>
          )}
        </div>
      )}
    </div>
  );
}
