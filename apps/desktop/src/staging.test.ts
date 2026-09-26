import { describe, expect, it } from "vitest";
import { fits, stagedError } from "./staging";

describe("staged errors", () => {
  it("tells the operator what is wanted, in words that read as an error", () => {
    const confirm = stagedError("confirm", "windows_powershell", "PS C:\\> ");
    expect(confirm[0]).toEqual({
      text: "The operation cannot continue until it is confirmed.",
      error: true,
    });
    expect(confirm.at(-1)).toEqual({ text: "PS C:\\> ", error: false });
    const resume = stagedError("resume", "pwsh", "PS C:\\> ");
    expect(resume[0].text).toContain("until the session is resumed");
    expect(resume.at(-1)?.text).toBe("PS C:\\> ");
  });

  it("looks like each shell's own errors, and never names KeyJutsu", () => {
    const ps5 = stagedError("confirm", "windows_powershell", "PS> ");
    expect(ps5.some((l) => l.text.includes("FullyQualifiedErrorId"))).toBe(true);
    expect(stagedError("confirm", "pwsh", "PS> ")[0].text).toMatch(/^OperationStopped: /);
    // cmd.exe prints its errors in the ordinary colour.
    expect(stagedError("confirm", "cmd", "C:\\>").every((l) => !l.error)).toBe(true);
    for (const shell of ["windows_powershell", "pwsh", "cmd"] as const) {
      for (const waiting of ["confirm", "resume"] as const) {
        const all = stagedError(waiting, shell, "> ")
          .map((l) => l.text)
          .join("\n");
        expect(all).not.toMatch(/keyjutsu/i);
      }
    }
  });

  it("only stages where it fits on screen", () => {
    expect(fits(0, 30, 5)).toBe(true);
    expect(fits(24, 30, 5)).toBe(true);
    expect(fits(25, 30, 5)).toBe(false);
  });
});
