import { useEffect, useRef, useState } from "react";
import type { workspace } from "@keyjutsu/types";
import { ipc } from "../ipc";

interface Props {
  view: workspace.ProfileView;
  onChange(view: workspace.ProfileView): void;
}

/**
 * Which work profile the next plan is made under, and why. Rust decides it
 * from the folder and the machine; this only shows the answer and passes on
 * what the operator chooses: a machine default, a folder confirmed, or a
 * profile picked by hand for this window.
 */
export function WorkProfile({ view, onChange }: Props) {
  const [picking, setPicking] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const run = (call: Promise<workspace.ProfileView>) =>
    call
      .then((v) => {
        setError(null);
        setPicking(false);
        onChange(v);
      })
      .catch((e: unknown) => setError(String(e)));

  return (
    <section className="work-profile" aria-label="Work profile">
      {view.state === "chosen" ? (
        <p className="work-profile-line">
          <span className="eyebrow">Work profile</span> <strong>{view.title}</strong>{" "}
          <span className="muted small">{view.why}</span>{" "}
          <button className="link" onClick={() => setPicking(true)}>
            Change…
          </button>
          {view.hand_picked && (
            <>
              {" · "}
              <button className="link" onClick={() => void run(ipc.profileClearPick())}>
                Go back to the default
              </button>
            </>
          )}
        </p>
      ) : (
        <div className="banner work-profile-needs">
          <p>
            <strong>
              {view.state === "no_default"
                ? "Choose a work profile"
                : view.state === "needs_confirmation"
                  ? "This folder asks for less"
                  : "No work profile can be chosen here"}
            </strong>{" "}
            {view.why}{" "}
            {view.state === "no_default"
              ? "It decides what the agent is told and what validation enforces, and it becomes this machine's default."
              : view.state === "needs_confirmation"
                ? `Use ${view.folder_profile ?? "it"} for this folder's files as they are now, or choose another profile.`
                : "Choose a profile for this window."}
          </p>
          <div className="row">
            {view.state === "needs_confirmation" && (
              <button className="primary" onClick={() => void run(ipc.profileConfirm())}>
                Use {view.folder_profile}
              </button>
            )}
            <button
              className={view.state === "needs_confirmation" ? "" : "primary"}
              onClick={() => setPicking(true)}
            >
              Choose a profile…
            </button>
          </div>
        </div>
      )}
      {error && <p className="banner error">{error}</p>}
      {picking && (
        <Picker
          view={view}
          onCancel={() => setPicking(false)}
          onPick={(name) => void run(ipc.profilePick(name))}
          onDefault={(name) => void run(ipc.profileSetDefault(name))}
        />
      )}
    </section>
  );
}

function Picker({
  view,
  onCancel,
  onPick,
  onDefault,
}: {
  view: workspace.ProfileView;
  onCancel(): void;
  onPick(name: string): void;
  onDefault(name: string): void;
}) {
  const [chosen, setChosen] = useState(
    view.name ?? view.machine_default ?? view.choices[0]?.name ?? "",
  );
  const first = useRef<HTMLInputElement>(null);
  useEffect(() => first.current?.focus(), []);
  // With nothing set, the answer is the machine's default; otherwise the
  // operator chooses whether it is for this window or from now on.
  const settingDefault = view.state === "no_default";
  const title = view.choices.find((c) => c.name === chosen)?.title ?? chosen;
  return (
    <div
      className="scrim"
      role="dialog"
      aria-modal="true"
      aria-label="Choose a work profile"
      onKeyDown={(e) => e.key === "Escape" && onCancel()}
    >
      <div className="card stack profile-picker">
        <h2>Choose a work profile</h2>
        <p className="small muted">
          Each one asks for at least what the one it extends asks for.
          {view.machine_default && ` This machine's default is ${view.machine_default}.`}
        </p>
        <fieldset className="profile-choices">
          <legend className="visually-hidden">Work profiles</legend>
          {view.choices.map((c, i) => (
            <label key={c.name} className="profile-choice">
              <input
                ref={i === 0 ? first : undefined}
                type="radio"
                name="work-profile"
                checked={chosen === c.name}
                onChange={() => setChosen(c.name)}
              />
              <span>
                <strong>{c.title}</strong>
                {c.extends && <span className="muted small"> · extends {c.extends}</span>}
                {c.summary && <span className="small block">{c.summary}</span>}
              </span>
            </label>
          ))}
        </fieldset>
        <div className="row end">
          <button onClick={onCancel}>Cancel</button>
          {!settingDefault && (
            <button disabled={!chosen} onClick={() => onPick(chosen)}>
              Use for this window
            </button>
          )}
          <button className="primary" disabled={!chosen} onClick={() => onDefault(chosen)}>
            {settingDefault ? `Use ${title}` : "Make it this machine's default"}
          </button>
        </div>
      </div>
    </div>
  );
}
