import type { technique } from "@keyjutsu/types";

/**
 * Parameters for a new Technique, one per line as `name = value`: the value
 * as it appears in the run's plan, which the Technique asks for next time.
 * Lines that are not a name and a value are ignored.
 */
export function parameterPairs(text: string): [string, string][] {
  return text
    .split("\n")
    .map((l) => l.split("="))
    .filter((p) => p.length === 2 && p[0].trim() && p[1].trim())
    .map(([n, v]) => [n.trim(), v.trim()]);
}

/** When it last changed: revalidated, or else saved. */
export const lastChanged = (t: technique.Technique) =>
  t.provenance.last_validated_at ?? t.provenance.created_at;

/** The most recently saved or revalidated first. */
export const latestFirst = (list: technique.Technique[]) =>
  [...list].sort((a, b) => lastChanged(b).localeCompare(lastChanged(a)));

/** Where it has worked, in words. An imported one has worked nowhere known here. */
export function whereItWorked(t: technique.Technique): string {
  const n = t.provenance.known_good.length;
  if (n === 0) return "not yet worked on a machine known here";
  return `worked on ${n} machine${n === 1 ? "" : "s"}`;
}

/**
 * Text from a Technique's plan with each `{{kj:name}}` shown as the value it
 * will get, or as `<name>` while that value is blank.
 */
export const filledIn = (text: string, values: Record<string, string>) =>
  text.replace(/\{\{kj:([^}]+)\}\}/g, (_, name: string) => values[name]?.trim() || `<${name}>`);
