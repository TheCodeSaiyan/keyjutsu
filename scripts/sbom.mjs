// A software bill of materials for what a release ships, in CycloneDX 1.5.
//
//   node scripts/sbom.mjs [OUT]
//
// Lists every Rust crate compiled into the three programs the installer
// carries (the desktop app, the CLI and the elevation broker) and every npm
// package bundled into the desktop app's window: normal dependencies only,
// for Windows x64. Test and build-time dependencies ship nothing and are
// left out. Each component carries its package URL, its licence as the
// package declares it, and for crates the SHA-256 Cargo.lock pins.
//
// Nothing is installed or fetched: cargo and pnpm answer from the lock files.
// OUT defaults to target/sbom/keyjutsu-VERSION.cdx.json.
import { execFileSync, execSync } from "node:child_process";
import { randomUUID } from "node:crypto";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const repo = join(dirname(fileURLToPath(import.meta.url)), "..");
const SHIPPED = ["keyjutsu-desktop", "keyjutsu-cli", "keyjutsu-broker"];

const options = { cwd: repo, encoding: "utf8", maxBuffer: 256 * 1024 * 1024 };
const run = (cmd, args) => execFileSync(cmd, args, options);
// pnpm is a .cmd script on Windows, which only a shell can start; its
// arguments here are fixed, never taken from input.
const pnpm = (args) => execSync(`pnpm ${args}`, options);

const version = JSON.parse(
  readFileSync(join(repo, "apps/desktop/src-tauri/tauri.conf.json"), "utf8"),
).version;

// Crates: the lock file's checksums, then a walk of normal dependencies
// from the three programs.
const checksums = new Map();
for (const block of readFileSync(join(repo, "Cargo.lock"), "utf8").split("[[package]]")) {
  const field = (k) => block.match(new RegExp(`^${k} = "([^"]*)"`, "m"))?.[1];
  if (field("checksum")) checksums.set(`${field("name")}@${field("version")}`, field("checksum"));
}
const meta = JSON.parse(
  run("cargo", [
    "metadata",
    "--format-version",
    "1",
    "--locked",
    "--filter-platform",
    "x86_64-pc-windows-msvc",
  ]),
);
const packages = new Map(meta.packages.map((p) => [p.id, p]));
const nodes = new Map(meta.resolve.nodes.map((n) => [n.id, n]));
const roots = meta.packages.filter((p) => SHIPPED.includes(p.name)).map((p) => p.id);
if (roots.length !== SHIPPED.length) {
  throw new Error(`expected ${SHIPPED.join(", ")} in the workspace, found ${roots.length}`);
}
const seen = new Set();
const queue = [...roots];
while (queue.length) {
  const id = queue.pop();
  if (seen.has(id)) continue;
  seen.add(id);
  for (const d of nodes.get(id)?.deps ?? []) {
    if (d.dep_kinds.some((k) => k.kind === null)) queue.push(d.pkg);
  }
}
const workspace = new Set(meta.workspace_members);
const crates = [...seen]
  .filter((id) => !workspace.has(id))
  .map((id) => packages.get(id))
  .map((p) => {
    const sum = checksums.get(`${p.name}@${p.version}`);
    return {
      type: "library",
      "bom-ref": `pkg:cargo/${p.name}@${p.version}`,
      name: p.name,
      version: p.version,
      purl: `pkg:cargo/${p.name}@${p.version}`,
      ...(p.license ? { licenses: [{ expression: p.license }] } : {}),
      ...(sum ? { hashes: [{ alg: "SHA-256", content: sum }] } : {}),
    };
  });

// npm: the desktop app's production dependencies, all the way down.
const [desktop] = JSON.parse(
  pnpm("--filter @keyjutsu/desktop list --prod --json --depth Infinity"),
);
const npm = new Map();
const walk = (deps) => {
  for (const [name, d] of Object.entries(deps ?? {})) {
    if (String(d.version).startsWith("link:")) {
      // A workspace package, bundled as source; its own dependencies ship.
      walk(d.dependencies);
      continue;
    }
    const key = `${name}@${d.version}`;
    if (!npm.has(key)) {
      let license;
      try {
        license = JSON.parse(readFileSync(join(d.path, "package.json"), "utf8")).license;
      } catch {
        // No readable package.json: listed without a licence.
      }
      const purl = `pkg:npm/${name.replace("@", "%40")}@${d.version}`;
      npm.set(key, {
        type: "library",
        "bom-ref": purl,
        name,
        version: d.version,
        purl,
        ...(typeof license === "string" ? { licenses: [{ expression: license }] } : {}),
      });
    }
    walk(d.dependencies);
  }
};
walk(desktop.dependencies);

const byPurl = (a, b) => a.purl.localeCompare(b.purl);
const bom = {
  bomFormat: "CycloneDX",
  specVersion: "1.5",
  serialNumber: `urn:uuid:${randomUUID()}`,
  version: 1,
  metadata: {
    timestamp: new Date().toISOString().replace(/\.\d+Z$/, "Z"),
    component: {
      type: "application",
      "bom-ref": `keyjutsu@${version}`,
      name: "KeyJutsu",
      version,
      licenses: [{ expression: "GPL-3.0-only" }],
    },
  },
  components: [...crates.sort(byPurl), ...[...npm.values()].sort(byPurl)],
};

const out = process.argv[2] ?? join(repo, "target", "sbom", `keyjutsu-${version}.cdx.json`);
mkdirSync(dirname(out), { recursive: true });
writeFileSync(out, `${JSON.stringify(bom, null, 2)}\n`);
console.log(`${out}: ${crates.length} crates, ${npm.size} npm packages`);
