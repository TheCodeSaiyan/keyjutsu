import { describe, expect, it } from "vitest";
import { modeLabel, ordinal, stateLabel } from "./labels";

describe("operator labels", () => {
  it("uses the design kit's canonical state names where it has them", () => {
    expect(stateLabel("EXECUTING")).toBe("Running");
    expect(stateLabel("AWAITING_USER_INPUT")).toBe("Waiting for user");
    expect(stateLabel("REVALIDATION_REQUIRED")).toBe("Revalidation required");
    expect(stateLabel("COMPLETE")).toBe("Complete");
  });

  it("names every mode", () => {
    expect(modeLabel("auto_performance")).toBe("Auto performance");
    expect(modeLabel("direct")).toBe("Direct");
  });

  it("pads step ordinals to two digits", () => {
    expect(ordinal(0)).toBe("01");
    expect(ordinal(11)).toBe("12");
  });
});
