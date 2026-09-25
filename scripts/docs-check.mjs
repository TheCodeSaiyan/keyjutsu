// Checks the documentation against the product, so the pages cannot drift
// from it quietly. It fails on:
//
// - a `keyjutsu ...` command, subcommand or option, in backticks or on a line
//   of a code block, that the built CLI does not have, read from its --help;
// - a relative link, or a #heading in one, that does not resolve;
// - a page under docs/ that nothing reachable from README.md links to;
// - hype words and American spellings in prose.
//
//   node scripts/docs-check.mjs
import { execFileSync } from "node:child_process";
import { existsSync, readFileSync, readdirSync, statSync } from "node:fs";
import { dirname, join, normalize, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const problems = [];
const report = (file, line, message) =>
  problems.push(`${relative(root, file)}:${line}: ${message}`);

// --- The pages -------------------------------------------------------------

// The design kit is its own documentation, kept as it was delivered.
const skip = [join("docs", "brand", "KeyJutsu-Design-Kit")];
function markdown(dir) {
  return readdirSync(dir).flatMap((name) => {
    const p = join(dir, name);
    if (skip.some((s) => relative(root, p).startsWith(s))) return [];
    if (statSync(p).isDirectory()) return markdown(p);
    return name.endsWith(".md") ? [p] : [];
  });
}
const topLevel = [
  "README.md",
  "THREAT_MODEL.md",
  "PRIVACY.md",
  "SECURITY.md",
  "CONTRIBUTING.md",
  "SUPPORT.md",
  "CODE_OF_CONDUCT.md",
]
  .map((f) => join(root, f))
  .filter(existsSync);
const pages = [...topLevel, ...markdown(join(root, "docs"))];

/** Lines outside fenced code blocks, with inline code kept for the command check. */
function prose(text) {
  let fenced = false;
  return text.split(/\r?\n/).map((line) => {
    if (/^\s*(```|~~~)/.test(line)) {
      fenced = !fenced;
      return "";
    }
    return fenced ? "" : line;
  });
}

// --- The CLI's own surface -------------------------------------------------

const exe = join(root, "target", "debug", "keyjutsu.exe");
if (!existsSync(exe)) {
  execFileSync("cargo", ["build", "-q", "-p", "keyjutsu-cli"], { cwd: root, stdio: "inherit" });
}
function help(args) {
  return execFileSync(exe, [...args, "--help"], { encoding: "utf8" });
}
function parseHelp(text) {
  const commands = [];
  const options = new Set(["--help", "-h"]);
  let section = "";
  for (const line of text.split(/\r?\n/)) {
    if (/^\S.*:$/.test(line)) section = line;
    else if (section === "Commands:") {
      const m = line.match(/^ {2}([a-z][a-z-]*)\s/);
      if (m && m[1] !== "help") commands.push(m[1]);
    } else if (section === "Options:") {
      for (const o of line.matchAll(/(?:^|[\s,])(--?[a-zA-Z][\w-]*)/g)) options.add(o[1]);
    }
  }
  return { commands, options };
}
// Every command path and the options it takes: "plan approve" -> {...}.
const tree = new Map();
function walk(path) {
  const parsed = parseHelp(help(path));
  tree.set(path.join(" "), parsed);
  for (const c of parsed.commands) walk([...path, c]);
}
walk([]);
tree.get("").options.add("--version").add("-V");

function checkCommand(file, line, span) {
  // A quoted argument is a value, such as a command to stage, not keyjutsu's.
  const unquoted = span.replace(/"[^"]*"|'[^']*'/g, "VALUE");
  const words = unquoted.trim().split(/\s+/).slice(1);
  let path = [];
  for (const w of words) {
    const node = tree.get(path.join(" "));
    if (node.commands.length === 0) break;
    if (!/^[a-z][a-z-]*$/.test(w)) break;
    if (!node.commands.includes(w)) {
      report(
        file,
        line,
        `\`${span}\`: keyjutsu ${path.join(" ")} has no command \`${w}\``.replace("  ", " "),
      );
      return;
    }
    path = [...path, w];
  }
  const node = tree.get(path.join(" "));
  for (const w of words) {
    const flag = w.split("=")[0];
    if (/^--?[a-zA-Z]/.test(flag) && !node.options.has(flag)) {
      report(file, line, `\`${span}\`: keyjutsu ${path.join(" ")} has no option \`${flag}\``);
    }
  }
}

// --- Links -----------------------------------------------------------------

function slug(heading) {
  return heading
    .trim()
    .toLowerCase()
    .replace(/`/g, "")
    .replace(/[^\p{L}\p{N}\s_-]/gu, "")
    .replace(/\s/g, "-");
}
const anchors = new Map();
function anchorsOf(file) {
  if (!anchors.has(file)) {
    const set = new Set();
    for (const line of prose(readFileSync(file, "utf8"))) {
      const m = line.match(/^#{1,6}\s+(.*)$/);
      if (m) set.add(slug(m[1]));
    }
    anchors.set(file, set);
  }
  return anchors.get(file);
}

const linkedFrom = new Map(); // page -> pages it links to
const hype =
  /\b(powerful|seamless(ly)?|comprehensive|simply|leverag(e|es|ed|ing)|effortless(ly)?|blazing)\b/i;
const american =
  /\b(behavior|behaviors|favorite|analyze[ds]?|analyzing|organize[ds]?|organizing|recognize[ds]?|licensed? to|color(s|ed)?|center(ed|s)?|catalog|initialize[ds]?|customize[ds]?|summarize[ds]?|prioritize[ds]?)\b/i;

/** Command lines inside fenced code blocks: `keyjutsu ...`, as typed. */
function fencedCommands(text) {
  let fenced = false;
  const found = [];
  text.split(/\r?\n/).forEach((line, i) => {
    if (/^\s*(```|~~~)/.test(line)) fenced = !fenced;
    else if (fenced) {
      const m = line.match(/^\s*(?:\$ |PS> )?(keyjutsu(?:\s[^#]*)?)/);
      if (m) found.push([i + 1, m[1].trim()]);
    }
  });
  return found;
}

for (const file of pages) {
  const text = readFileSync(file, "utf8");
  for (const [line, command] of fencedCommands(text)) checkCommand(file, line, command);
  const lines = prose(text);
  const targets = new Set();
  lines.forEach((text, i) => {
    const line = i + 1;
    for (const m of text.matchAll(/`(keyjutsu(?: [^`]*)?)`/g)) checkCommand(file, line, m[1]);
    const words = text
      .replace(/`[^`]*`/g, "")
      .replace(/<[^>]*>/g, "")
      .replace(/\]\([^)]*\)/g, "]");
    const h = words.match(hype);
    if (h) report(file, line, `"${h[0]}": say what it stands in for`);
    // A quotation keeps its own spelling: Windows says "not recognized".
    const a = words.replace(/"[^"]*"|“[^”]*”/g, "").match(american);
    if (a) report(file, line, `"${a[0]}": British spelling`);
    for (const m of text.matchAll(/\]\(([^)\s]+)\)/g)) {
      const href = m[1];
      if (/^(https?:|mailto:)/.test(href)) continue;
      const [path, fragment] = href.split("#");
      const target = path ? normalize(join(dirname(file), decodeURIComponent(path))) : file;
      if (!existsSync(target)) {
        report(file, line, `link to ${href}: no such file`);
        continue;
      }
      if (target.endsWith(".md")) {
        targets.add(target);
        if (fragment && !anchorsOf(target).has(fragment))
          report(file, line, `link to ${href}: no such heading`);
      } else if (statSync(target).isDirectory() && existsSync(join(target, "README.md"))) {
        targets.add(join(target, "README.md"));
      }
    }
  });
  linkedFrom.set(file, targets);
}

// --- Orphans ---------------------------------------------------------------

const reached = new Set();
const queue = [join(root, "README.md")];
while (queue.length) {
  const p = queue.pop();
  if (reached.has(p)) continue;
  reached.add(p);
  for (const t of linkedFrom.get(p) ?? []) queue.push(t);
}
for (const p of pages) {
  if (!reached.has(p)) {
    report(p, 1, "nothing reachable from README.md links to this page");
  }
}

if (problems.length) {
  console.log(problems.join("\n"));
  console.log(`\n${problems.length} problem(s) in ${pages.length} pages.`);
  process.exit(1);
}
const commands = [...tree.keys()].filter(Boolean).length;
console.log(
  `Docs check: ${pages.length} pages, links, headings and ${commands} keyjutsu commands all agree.`,
);
