// Builds the CLI and the elevation broker and places them where Tauri
// bundles them next to the desktop app (tauri.conf.json: bundle.externalBin).
// The installer puts them in the install folder as keyjutsu.exe and
// keyjutsu-broker.exe, where the app and the CLI look for the broker.
//
// Tauri wants each sidecar named with the target triple, so it is read from
// rustc rather than assumed.
import { execFileSync } from "node:child_process";
import { copyFileSync, mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const triple = execFileSync("rustc", ["-vV"], { encoding: "utf8" })
  .match(/^host: (.+)$/m)[1]
  .trim();
execFileSync("cargo", ["build", "--release", "-p", "keyjutsu-cli", "-p", "keyjutsu-broker"], {
  cwd: root,
  stdio: "inherit",
});
const out = join(root, "apps", "desktop", "src-tauri", "binaries");
mkdirSync(out, { recursive: true });
for (const name of ["keyjutsu", "keyjutsu-broker"]) {
  copyFileSync(join(root, "target", "release", `${name}.exe`), join(out, `${name}-${triple}.exe`));
  console.log(`staged ${name}-${triple}.exe`);
}
