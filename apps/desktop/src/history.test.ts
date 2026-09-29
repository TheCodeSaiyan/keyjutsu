import { describe, expect, it } from "vitest";
import type { history } from "@keyjutsu/types";
import {
  agentLabel,
  duration,
  newestFirst,
  outcomeBadge,
  stepTitles,
  stoppedBecause,
  when,
} from "./history";

describe("history", () => {
  it("shows each outcome as a badge", () => {
    expect(outcomeBadge("complete")).toEqual({ label: "Completed", tone: "ready" });
    expect(outcomeBadge("failed at build-pdf")).toEqual({ label: "Failed", tone: "blocked" });
    expect(outcomeBadge("disarmed").tone).toBe("none");
    expect(outcomeBadge("waiting: Windows restart").label).toBe("Waiting");
  });

  it("gives times in the operator's clock and durations in words", () => {
    expect(when("2026-09-28T16:46:06Z", "en-GB", "Europe/London")).toBe("Mon 28 Sept, 17:46");
    expect(when("not a date")).toBe("not a date");
    expect(duration("2026-09-28T16:00:00Z", "2026-09-28T16:00:45Z")).toBe("45 s");
    expect(duration("2026-09-28T16:00:00Z", "2026-09-28T16:00:00Z")).toBe("under a second");
    expect(duration("2026-09-28T16:00:00Z", "2026-09-28T16:02:14Z")).toBe("2 min 14 s");
    expect(duration("2026-09-28T16:00:00Z", "2026-09-28T17:03:00Z")).toBe("1 h 3 min");
  });

  it("names steps by title, and says why a run stopped", () => {
    const titles = stepTitles(
      JSON.stringify({ plan: { steps: [{ id: "build-pdf", title: "Build the PDF" }] } }),
    );
    expect(titles.get("build-pdf")).toBe("Build the PDF");
    expect(stepTitles("not json").size).toBe(0);
    expect(stoppedBecause({ kind: "complete" }, titles)).toBeNull();
    expect(
      stoppedBecause(
        {
          kind: "failed",
          step: "build-pdf",
          expected: "exit code 0",
          actual: "exit code 1",
          output: "",
        },
        titles,
      ),
    ).toBe("“Build the PDF” failed: expected exit code 0, got exit code 1.");
    expect(
      stoppedBecause({ kind: "aborted", step: "build-pdf", in_doubt: true }, titles),
    ).toContain("“Build the PDF” after its command was sent");
  });

  it("lists the newest first and names the agent", () => {
    const runs = [
      {
        id: "a",
        finished_at: "2026-09-27T10:00:00Z",
        task: "t",
        outcome: "complete",
        recorded: false,
      },
      {
        id: "b",
        finished_at: "2026-09-28T10:00:00Z",
        task: "t",
        outcome: "complete",
        recorded: false,
      },
    ] satisfies history.SessionSummary[];
    expect(newestFirst(runs).map((r) => r.id)).toEqual(["b", "a"]);
    expect(agentLabel({ name: "claude_code", version: "2.1.283" })).toBe("Claude Code 2.1.283");
  });
});
