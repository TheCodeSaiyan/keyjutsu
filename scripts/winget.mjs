// The winget manifests for a release, ready to submit to microsoft/winget-pkgs.
//
//   node scripts/winget.mjs 0.2.0 path/to/KeyJutsu_0.2.0_x64-setup.exe out/winget
//
// The release workflow runs this on the installer it is about to publish, so
// the hash in the manifest is of that very file, and keeps the result as a
// workflow artifact. Submitting is a separate, deliberate step: see
// "Releasing" in CONTRIBUTING.md. winget downloads from InstallerUrl, which
// has to be a public release for anyone else to install from it.
//
// Written out by hand rather than with a YAML library: three short files of
// scalars and short lists, and every value here is ours, not user input.
import { createHash } from "node:crypto";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { basename, dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const [version, installer, out] = process.argv.slice(2);
if (!/^\d+\.\d+\.\d+$/.test(version ?? "") || !installer || !out) {
  console.error("usage: node scripts/winget.mjs X.Y.Z INSTALLER.exe OUT_DIR");
  process.exit(2);
}
const expected = `KeyJutsu_${version}_x64-setup.exe`;
if (basename(installer) !== expected) {
  console.error(`the installer for ${version} is called ${expected}, not ${basename(installer)}`);
  process.exit(1);
}

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const conf = JSON.parse(readFileSync(join(root, "apps/desktop/src-tauri/tauri.conf.json"), "utf8"));
if (conf.version !== version) {
  console.error(
    `the code says ${conf.version}, not ${version}; run node scripts/version.mjs set ${version}`,
  );
  process.exit(1);
}

const id = "TheCodeSaiyan.KeyJutsu";
const repo = "https://github.com/TheCodeSaiyan/keyjutsu";
const schema = "1.9.0";
const sha256 = createHash("sha256").update(readFileSync(installer)).digest("hex").toUpperCase();
const head = (type) =>
  `# yaml-language-server: $schema=https://aka.ms/winget-manifest.${type}.${schema}.schema.json\n\n` +
  `PackageIdentifier: ${id}\nPackageVersion: ${version}\n`;
const tail = (type) => `ManifestType: ${type}\nManifestVersion: ${schema}\n`;

const files = {
  [`${id}.yaml`]: head("version") + "DefaultLocale: en-GB\n" + tail("version"),

  // The installer is Tauri's NSIS one, installed for every account, and it
  // asks Windows for Administrator itself. /S answers yes to its two
  // questions (PATH, and "Open KeyJutsu here" in Explorer). Apps & Features
  // lists it under the publisher in tauri.conf.json, which is how winget
  // recognises it as installed.
  [`${id}.installer.yaml`]:
    head("installer") +
    [
      "Platform:\n- Windows.Desktop",
      "MinimumOSVersion: 10.0.22000.0",
      "InstallerType: nullsoft",
      "Scope: machine",
      "InstallModes:\n- interactive\n- silent",
      "UpgradeBehavior: install",
      "ElevationRequirement: elevatesSelf",
      "Commands:\n- keyjutsu",
      "AppsAndFeaturesEntries:\n" +
        `- DisplayName: ${conf.productName}\n` +
        `  Publisher: ${conf.bundle.publisher}\n` +
        `  DisplayVersion: ${version}`,
      "Installers:\n" +
        "- Architecture: x64\n" +
        `  InstallerUrl: ${repo}/releases/download/v${version}/${expected}\n` +
        `  InstallerSha256: ${sha256}`,
    ].join("\n") +
    "\n" +
    tail("installer"),

  [`${id}.locale.en-GB.yaml`]:
    head("defaultLocale") +
    [
      "PackageLocale: en-GB",
      `Publisher: ${conf.bundle.publisher}`,
      "PublisherUrl: https://github.com/TheCodeSaiyan",
      `PackageName: ${conf.productName}`,
      `PackageUrl: ${repo}`,
      "License: GPL-3.0-only",
      `LicenseUrl: ${repo}/blob/main/LICENSE`,
      `ShortDescription: ${conf.bundle.shortDescription}`,
      "Description: Validated commands, theatrically typed. An AI agent you already use proposes a plan; " +
        "KeyJutsu validates each step without running it, you approve it, and the approved commands " +
        "type themselves into a real shell while you mash keys.",
      "Moniker: keyjutsu",
      "Tags:\n- terminal\n- powershell\n- ai-agents\n- automation",
      `ReleaseNotesUrl: ${repo}/releases/tag/v${version}`,
    ].join("\n") +
    "\n" +
    tail("defaultLocale"),
};

// The layout winget-pkgs uses, so the folder can be copied in as it is.
const dir = join(out, "manifests", "t", "TheCodeSaiyan", "KeyJutsu", version);
mkdirSync(dir, { recursive: true });
for (const [name, text] of Object.entries(files)) {
  writeFileSync(join(dir, name), text);
}
console.log(`Wrote the winget manifests for ${version} to ${dir}`);
console.log(`InstallerSha256 ${sha256}, of ${installer}`);
