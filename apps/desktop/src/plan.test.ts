import { describe, expect, it } from "vitest";
import type { plan } from "@keyjutsu/types";
import {
  boundaryStep,
  retryGuidance,
  formFromStep,
  idFor,
  linesToCommands,
  newStep,
  outcomeLine,
  readinessLabel,
  readinessTone,
  riskLabel,
  stepFromForm,
} from "./plan";

const base: plan.Step = {
  id: "fix",
  title: "Fix it",
  objective: "Repair.",
  kind: "command",
  shell: { kind: "pwsh" },
  commands: [{ text: "Restart-Service -Name docker", purpose: "restart" }],
  tool_requirements: [],
  depends_on: ["look"],
  preconditions: [],
  privilege: "administrator",
  expected_effects: [],
  visible_validation: [],
  internal_validation: [{ exit_code: { equals: 0n } }],
  artifacts: [],
};

describe("labels", () => {
  it("uses the kit's state words and never colour alone", () => {
    expect(readinessLabel("READY", true)).toBe("Ready");
    expect(readinessLabel("NEEDS_REVIEW", true)).toBe("Review");
    expect(readinessTone("INVALID")).toBe("blocked");
  });

  it("says a changed step needs revalidation, and a new plan needs validation", () => {
    expect(readinessLabel(undefined, true)).toBe("Revalidation required");
    expect(readinessLabel(undefined, false)).toBe("Not validated");
  });

  it("keeps risk separate from readiness", () => {
    expect(riskLabel("high")).toBe("High");
    expect(riskLabel(undefined)).toBe("Not assessed");
  });
});

describe("editing", () => {
  it("drops blank lines but keeps each command exactly as typed", () => {
    expect(linesToCommands("a\n\n  b -X 'c'  \r\n")).toEqual([
      { text: "a" },
      { text: "  b -X 'c'  " },
    ]);
  });

  it("changes only what the form shows", () => {
    const f = formFromStep(base);
    const next = stepFromForm(base, { ...f, commands: "Start-Service -Name docker" });
    expect(next.commands).toEqual([{ text: "Start-Service -Name docker" }]);
    expect(next.privilege).toBe("administrator");
    expect(next.internal_validation).toEqual(base.internal_validation);
    expect(next.depends_on).toEqual(["look"]);
  });

  it("adds a commands recovery, and removes it when emptied", () => {
    const withRecovery = stepFromForm(base, {
      ...formFromStep(base),
      recoveryCommands: "Start-Service -Name docker",
    });
    expect(withRecovery.recovery?.strategy).toBe("commands");
    const without = stepFromForm(withRecovery, {
      ...formFromStep(withRecovery),
      recoveryCommands: "",
    });
    expect(without.recovery).toBeUndefined();
  });

  it("leaves a captured-state recovery alone", () => {
    const captured: plan.Step = {
      ...base,
      recovery: {
        strategy: "restore_captured_state",
        capture: [{ kind: "file", target: "C:/x" }],
        commands: [],
        validation: [],
      },
    };
    expect(stepFromForm(captured, formFromStep(captured)).recovery).toEqual(captured.recovery);
  });

  it("makes manual steps without commands or shell", () => {
    const s = newStep("manual", "look", "Look at the tray", "It is steady.", "ignored", {
      kind: "pwsh",
    });
    expect(s.commands).toEqual([]);
    expect(s.shell).toBeUndefined();
  });

  it("makes ids from titles that the schema accepts and that are not taken", () => {
    expect(idFor("Check the tray!", [])).toBe("check-the-tray");
    expect(idFor("Check the tray", ["check-the-tray"])).toBe("check-the-tray-2");
    expect(idFor("---", [])).toBe("step");
    expect(idFor("Über", [])).toMatch(/^[a-z0-9][a-z0-9_-]*$/);
  });
});

describe("outcomes", () => {
  it("says what was expected and what happened", () => {
    expect(
      outcomeLine({ kind: "failed", step: "b", expected: "exit 0", actual: "exit 3" }),
    ).toContain("Expected exit 0; got exit 3");
    expect(outcomeLine({ kind: "aborted", step: "b", in_doubt: true })).toContain("unknown");
  });

  it("says how to carry on after each kind of boundary, in the app", () => {
    expect(outcomeLine({ kind: "boundary", phase: "install", boundary: "windows_restart" })).toBe(
      "Phase install is done. The plan waits for a Windows restart.",
    );
    const all: plan.Boundary[] = [
      "windows_restart",
      "sign_out",
      "shell_restart",
      "wsl_restart",
      "docker_restart",
    ];
    for (const b of all) {
      expect(boundaryStep(b)).toMatch(/continue/);
      expect(boundaryStep(b)).not.toMatch(/--resume/);
    }
    // A restart or sign-out closes the app: it offers to continue when it opens again.
    expect(boundaryStep("windows_restart")).toContain("open KeyJutsu");
  });
});

describe("retryGuidance", () => {
  const state = (evidence: { check: string; result: string; detail?: string }[]) =>
    ({ evidence }) as unknown as plan.StepState;

  it("writes what validation found, once each, for the agent", () => {
    const g = retryGuidance(
      state([
        { check: "commands", result: "failed", detail: "`magick` was not found" },
        { check: "commands", result: "failed", detail: "`magick` was not found" },
        { check: "syntax", result: "passed", detail: "parses" },
      ]),
    );
    expect(g).toContain("- commands: `magick` was not found");
    expect(g.match(/magick/g)?.length).toBe(1);
    expect(g).not.toContain("parses");
  });

  it("leaves out what staging fixes, and is empty when nothing failed", () => {
    expect(
      retryGuidance(state([{ check: "artifact", result: "failed", detail: "not pinned yet" }])),
    ).toBe("");
    expect(retryGuidance(undefined)).toBe("");
  });
});
