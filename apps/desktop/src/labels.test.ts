import { describe, expect, it } from "vitest";
import { elapsed, modeLabel, ordinal, stateLabel } from "./labels";

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

describe("elapsed", () => {
  it("reads as a clock", () => {
    expect(elapsed(0)).toBe("0:00");
    expect(elapsed(7.9)).toBe("0:07");
    expect(elapsed(754)).toBe("12:34");
    expect(elapsed(3725)).toBe("1:02:05");
    expect(elapsed(-3)).toBe("0:00");
  });
});
