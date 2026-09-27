import { useState } from "react";
import type { update } from "@keyjutsu/types";
import { ipc } from "../ipc";

/**
 * Checking for, and installing, a newer KeyJutsu. Nothing is asked of
 * GitHub until the operator presses the button, and the update is the
 * signed installer, checked before it runs; Rust refuses it while a plan is
 * running.
 */
export function UpdatePanel() {
  const [channel, setChannel] = useState<update.Channel>("stable");
  const [checked, setChecked] = useState<update.Checked | null>(null);
  const [working, setWorking] = useState<"checking" | "installing" | null>(null);
  const [error, setError] = useState<string | null>(null);

  const check = async () => {
    setError(null);
    setWorking("checking");
    try {
      setChecked(await ipc.updateCheck(channel));
    } catch (e) {
      setError(String(e));
    } finally {
      setWorking(null);
    }
  };

  const install = async () => {
    setError(null);
    setWorking("installing");
    try {
      await ipc.updateInstall();
    } catch (e) {
      setError(String(e));
      setWorking(null);
    }
  };

  const available = checked?.available ?? null;
  return (
    <div className="stack">
      <p className="small muted">
        KeyJutsu asks GitHub for a newer version only when you press this. An update is the signed
        installer, checked before it runs, and never while a plan is running.
      </p>
      <div className="row">
        <label className="small">
          Channel{" "}
          <select
            value={channel}
            disabled={working !== null}
            onChange={(e) => {
              setChannel(e.target.value as update.Channel);
              setChecked(null);
            }}
          >
            <option value="stable">Releases</option>
            <option value="beta">Pre-releases too</option>
          </select>
        </label>
        <button disabled={working !== null} onClick={() => void check()}>
          Check for updates
        </button>
      </div>
      {working === "checking" && (
        <p role="status" className="small muted">
          Asking GitHub…
        </p>
      )}
      {checked && !available && (
        <p role="status" className="small">
          KeyJutsu {checked.current} is the newest.
        </p>
      )}
      {available && (
        <div className="stack">
          <p role="status" className="small">
            KeyJutsu {available.version}
            {available.prerelease ? ", a pre-release," : ""} is available. This is{" "}
            {checked?.current}.
          </p>
          {available.notes.trim() && (
            <pre
              className="code bundle"
              tabIndex={0}
              aria-label={`What changed in ${available.version}`}
            >
              {available.notes.trim()}
            </pre>
          )}
          <div className="row">
            <button className="primary" disabled={working !== null} onClick={() => void install()}>
              Install {available.version}
            </button>
          </div>
          <p className="small muted">
            KeyJutsu downloads the installer, checks it against the release&apos;s checksums and its
            signature, then starts it and closes. Windows asks for Administrator.
          </p>
        </div>
      )}
      {working === "installing" && (
        <p role="status" className="small muted">
          Downloading and checking the installer…
        </p>
      )}
      {error && (
        <p role="alert" className="error">
          {error}
        </p>
      )}
    </div>
  );
}
