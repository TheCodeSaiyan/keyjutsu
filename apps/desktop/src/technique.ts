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
