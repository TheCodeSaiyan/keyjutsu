import type { CheckStatus, ReadinessReport } from "@keyjutsu/types";
import { ipc } from "../ipc";

// Status is spelt out as text and a glyph, never colour alone.
const LABEL: Record<CheckStatus, { glyph: string; text: string }> = {
  ok: { glyph: "✓", text: "Ready" },
  warning: { glyph: "!", text: "Check" },
  unavailable: { glyph: "✕", text: "Unavailable" },
};

/**
 * The machine's readiness checks. `fold` keeps what needs a look in view and
 * folds what is ready behind one line, for a panel that is not about them.
 */
export function ReadinessPanel({
  report,
  fold = false,
}: {
  report: ReadinessReport | null;
  fold?: boolean;
}) {
  if (!report) {
    return (
      <p className="muted" role="status">
        Scanning this machine: starting each shell once in a throwaway terminal…
      </p>
    );
  }
  if (!fold) return <Checks checks={report.checks} />;
  const attention = report.checks.filter((c) => c.status !== "ok");
  const ready = report.checks.filter((c) => c.status === "ok");
  return (
    <>
      {attention.length > 0 && <Checks checks={attention} />}
      {ready.length > 0 && (
        <details className="checks-ready">
          <summary className="small">
            <span aria-hidden="true">✓</span> {ready.length}{" "}
            {ready.length === 1 ? "check" : "checks"} ready
          </summary>
          <Checks checks={ready} />
        </details>
      )}
    </>
  );
}

function Checks({ checks }: { checks: ReadinessReport["checks"] }) {
  return (
    <table className="checks">
      <caption className="visually-hidden">Readiness checks</caption>
      <tbody>
        {checks.map((c) => (
          <tr key={c.name} data-status={c.status}>
            <td className="check-glyph" aria-hidden="true">
              {LABEL[c.status].glyph}
            </td>
            <th scope="row">{c.name}</th>
            <td>
              <span className="visually-hidden">{LABEL[c.status].text}: </span>
              {c.detail}
              {c.get && (
                <>
                  {" "}
                  <button
                    className="link"
                    onClick={() => {
                      if (c.get) void ipc.openLink(c.get.url);
                    }}
                  >
                    {c.get.label}
                  </button>
                </>
              )}
            </td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}
