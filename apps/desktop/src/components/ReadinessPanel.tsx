import type { CheckStatus, ReadinessReport } from "@keyjutsu/types";

// Status is spelt out as text and a glyph, never colour alone.
const LABEL: Record<CheckStatus, { glyph: string; text: string }> = {
  ok: { glyph: "✓", text: "Ready" },
  warning: { glyph: "!", text: "Check" },
  unavailable: { glyph: "✕", text: "Unavailable" },
};

export function ReadinessPanel({ report }: { report: ReadinessReport | null }) {
  if (!report) {
    return (
      <p className="muted" role="status">
        Scanning this machine: starting each shell once in a throwaway terminal…
      </p>
    );
  }
  return (
    <table className="checks">
      <caption className="visually-hidden">Readiness checks</caption>
      <tbody>
        {report.checks.map((c) => (
          <tr key={c.name} data-status={c.status}>
            <td className="check-glyph" aria-hidden="true">
              {LABEL[c.status].glyph}
            </td>
            <th scope="row">{c.name}</th>
            <td>
              <span className="visually-hidden">{LABEL[c.status].text}: </span>
              {c.detail}
            </td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}
