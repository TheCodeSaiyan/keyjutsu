// KeyJutsu's version, which is written in several places that must agree.
//
//   node scripts/version.mjs            check they agree (CI runs this)
//   node scripts/version.mjs set 0.2.0  set them all, for a release commit
//
// The workspace Cargo.toml is the source: the CLI, the broker and the desktop
// app all take their version from it. The rest repeat it because their tools
// cannot read it: Tauri names the installer from tauri.conf.json, pnpm reads
// the package.json files, and the install guides quote the installer's name.
// A release where they disagree ships an installer whose name, About box and
// `keyjutsu --version` say three different things.
import { execFileSync } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const read = (f) => readFileSync(join(root, f), "utf8");
const semver = /^\d+\.\d+\.\d+$/;

// Each place: how to find the version in it, and how to put a new one in.
const places = [
  {
    file: "Cargo.toml",
    find: (t) => t.match(/\[workspace\.package\][^[]*?\nversion = "([^"]+)"/)?.[1],
    set: (t, v) => t.replace(/(\[workspace\.package\][^[]*?\nversion = ")[^"]+(")/, `$1${v}$2`),
  },
  ...[
    "apps/desktop/src-tauri/tauri.conf.json",
    "apps/desktop/package.json",
    "packages/types/package.json",
  ].map((file) => ({
    file,
    find: (t) => JSON.parse(t).version,
    set: (t, v) => t.replace(/("version": ")[^"]+(")/, `$1${v}$2`),
  })),
  // The installer's and portable build's names, and `keyjutsu doctor`'s first line, as the guides
  // quote them. Every mention on the page must match.
  ...["docs/guides/installing.md", "docs/plain/installing.md"].map((file) => ({
    file,
    find: (t) => {
      const seen = new Set(
        [
          ...t.matchAll(
            /KeyJutsu_(\d+\.\d+\.\d+)_x64-(?:setup\.exe|portable\.zip)|^KeyJutsu (\d+\.\d+\.\d+)$/gm,
          ),
        ].map((m) => m[1] ?? m[2]),
      );
      return seen.size === 1 ? [...seen][0] : seen.size === 0 ? undefined : [...seen].join(" and ");
    },
    set: (t, v) =>
      t
        .replace(/KeyJutsu_\d+\.\d+\.\d+_x64-(setup\.exe|portable\.zip)/g, `KeyJutsu_${v}_x64-$1`)
        .replace(/^KeyJutsu \d+\.\d+\.\d+$/gm, `KeyJutsu ${v}`),
  })),
];

const [command, wanted] = process.argv.slice(2);
if (command === "set") {
  if (!semver.test(wanted ?? "")) {
    console.error("usage: node scripts/version.mjs set X.Y.Z");
    process.exit(2);
  }
  for (const p of places) writeFileSync(join(root, p.file), p.set(read(p.file), wanted));
  // Cargo.lock records each workspace crate's version too.
  execFileSync("cargo", ["update", "--workspace", "--offline", "--quiet"], {
    cwd: root,
    stdio: "inherit",
  });
}

const found = places.map((p) => ({ file: p.file, version: p.find(read(p.file)) }));
const expected = found[0].version;
const wrong = found.filter((f) => f.version !== expected);
for (const f of found)
  console.log(`${f.version === expected ? "ok  " : "FAIL"}  ${f.version ?? "missing"}  ${f.file}`);
if (wrong.length) {
  console.error(
    `\nThe version is ${expected} in Cargo.toml, and something else in ${wrong.length} place(s).`,
  );
  process.exit(1);
}
console.log(`\nVersion ${expected}, the same everywhere.`);
