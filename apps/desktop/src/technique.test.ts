import { describe, expect, it } from "vitest";
import { parameterPairs } from "./technique";

describe("parameterPairs", () => {
  it("reads one name = value per line, trimmed", () => {
    expect(parameterPairs("service_name = Winmgmt\n  folder=C:/Users/Public  ")).toEqual([
      ["service_name", "Winmgmt"],
      ["folder", "C:/Users/Public"],
    ]);
  });

  it("ignores lines that are not a name and a value", () => {
    expect(parameterPairs("\njust words\n= no name\nno value =\na = b = c")).toEqual([]);
  });
});
