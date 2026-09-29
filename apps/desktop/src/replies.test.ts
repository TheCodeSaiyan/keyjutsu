import { describe, expect, it } from "vitest";
import { nextBatch, type Reply } from "./replies";

let id = 0;
const reply = (
  step: string | null,
  text: string,
  agent: Reply["agent"] = "claude_code",
): Reply => ({
  id: ++id,
  agent,
  step,
  stepTitle: step ? `Title of ${step}` : null,
  text,
});

describe("nextBatch", () => {
  it("is nothing when nothing is queued", () => {
    expect(nextBatch([], ["a"])).toBeNull();
  });

  it("sends replies in a row about the same thing as one request", () => {
    const q = [reply("a", "use D:"), reply("a", "and keep the log"), reply(null, "shorter plan")];
    const b = nextBatch(q, ["a"])!;
    expect(b.step).toBe("a");
    expect(b.guidance).toBe("use D:\n\nand keep the log");
    expect(b.replies.map((r) => r.id)).toEqual([q[0].id, q[1].id]);
  });

  it("keeps the order: a later reply about the same step waits behind one about the plan", () => {
    const q = [reply("a", "first"), reply(null, "second"), reply("a", "third")];
    expect(nextBatch(q, ["a"])!.replies).toEqual([q[0]]);
  });

  it("does not mix agents", () => {
    const q = [reply(null, "one"), reply(null, "two", "codex")];
    expect(nextBatch(q, [])!.replies).toEqual([q[0]]);
  });

  it("sends a reply about a step that has gone to the plan, naming the step", () => {
    const b = nextBatch([reply("gone", "check the PDF")], ["a"])!;
    expect(b.step).toBeNull();
    expect(b.guidance).toBe('About the step "Title of gone", since removed: check the PDF');
  });
});
