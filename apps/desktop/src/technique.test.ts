import { describe, expect, it } from "vitest";
import type { technique } from "@keyjutsu/types";
import { filledIn, lastChanged, latestFirst, parameterPairs, whereItWorked } from "./technique";

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

const saved = (
  id: string,
  created_at: string,
  extra: Partial<technique.TechniqueProvenance> = {},
  known_good = 1,
) =>
  ({
    id,
    provenance: {
      created_at,
      imported: false,
      known_good: Array.from({ length: known_good }, () => ({})),
      ...extra,
    },
  }) as unknown as technique.Technique;

describe("the Techniques list", () => {
  it("puts the most recently saved or revalidated first", () => {
    const list = [
      saved("old", "2026-09-01T10:00:00Z"),
      saved("revised", "2026-08-01T10:00:00Z", { last_validated_at: "2026-09-20T10:00:00Z" }),
      saved("new", "2026-09-10T10:00:00Z"),
    ];
    expect(latestFirst(list).map((t) => t.id)).toEqual(["revised", "new", "old"]);
    expect(list.map((t) => t.id)).toEqual(["old", "revised", "new"]);
  });

  it("says when it last changed", () => {
    expect(lastChanged(saved("a", "2026-09-01T10:00:00Z"))).toBe("2026-09-01T10:00:00Z");
    expect(
      lastChanged(
        saved("b", "2026-09-01T10:00:00Z", { last_validated_at: "2026-09-02T10:00:00Z" }),
      ),
    ).toBe("2026-09-02T10:00:00Z");
  });

  it("says how many machines it worked on, in words", () => {
    expect(whereItWorked(saved("a", "2026-09-01T10:00:00Z", {}, 1))).toBe("worked on 1 machine");
    expect(whereItWorked(saved("a", "2026-09-01T10:00:00Z", {}, 3))).toBe("worked on 3 machines");
    expect(whereItWorked(saved("a", "2026-09-01T10:00:00Z", { imported: true }, 0))).toBe(
      "not yet worked on a machine known here",
    );
  });
});

describe("filledIn", () => {
  it("shows each placeholder as the value it will get", () => {
    expect(filledIn("Look at the {{kj:service_name}} service", { service_name: "Winmgmt" })).toBe(
      "Look at the Winmgmt service",
    );
    expect(filledIn("{{kj:a}} then {{kj:a}} and {{kj:b}}", { a: "x", b: "y" })).toBe(
      "x then x and y",
    );
  });

  it("names a parameter that has no value yet", () => {
    expect(filledIn("Look at the {{kj:service_name}} service", { service_name: "  " })).toBe(
      "Look at the <service_name> service",
    );
    expect(filledIn("Look at the {{kj:service_name}} service", {})).toBe(
      "Look at the <service_name> service",
    );
  });

  it("leaves text without placeholders alone", () => {
    expect(filledIn("Say when Windows last started", { a: "x" })).toBe(
      "Say when Windows last started",
    );
  });
});
