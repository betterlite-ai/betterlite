import fs from "node:fs/promises";
import path from "node:path";

import { PatchError } from "../patches.mjs";
import { listTree, treeDigest } from "./inputs.mjs";

const READY_MARKER = ".litellm_ui_ready";
const LOGOS = new Set(["logo.png", "logo_dark.png", "logo_monogram.png", "logo_monogram_dark.png", "cached_logo.jpg"]);
const LICENSE_FILE = /^(licen[cs]e|notice|copying|copyright)(\.|$)/i;
const HEX = /^[0-9a-f]{64}$/;
const ROUTE_GROUP = /^\([^.][^)]*\)$/;

async function appRoutes(appDir) {
  const routes = new Set();
  const pending = [""];
  while (pending.length > 0) {
    const relative = pending.pop();
    for (const entry of await fs.readdir(path.join(appDir, relative), { withFileTypes: true })) {
      if (entry.isDirectory()) {
        pending.push(relative === "" ? entry.name : `${relative}/${entry.name}`);
      } else if (/^page\.(tsx|ts|jsx|js)$/.test(entry.name)) {
        const segments = relative.split("/").filter((segment) => segment !== "" && !ROUTE_GROUP.test(segment));
        if (!segments.some((segment) => /^([[@_]|\(\.)/.test(segment))) routes.add(segments.join("/"));
      }
    }
  }
  return [...routes].sort();
}

async function verifyLayout(files, appRoot) {
  const have = new Set(files.map((file) => file.path));
  const required = ["index.html", "login/index.html"];
  for (const route of await appRoutes(path.join(appRoot, "src/app"))) {
    required.push(route === "" ? "index.html" : `${route}/index.html`);
  }
  const publicDir = path.join(appRoot, "public");
  if (await fs.stat(publicDir).catch(() => null)) {
    for (const asset of await listTree(publicDir)) required.push(asset.path);
  }
  const missing = [...new Set(required)].filter((relative) => !have.has(relative));
  if (!files.some((file) => file.path.startsWith("_next/static/"))) missing.push("_next/static/");
  if (!files.some((file) => file.path.endsWith(".txt"))) missing.push("RSC *.txt payloads");
  if (!files.some((file) => file.path.endsWith(".txt") && file.path.split("/").some((part) => part.startsWith("__next")))) {
    missing.push("__next segment files");
  }
  if (missing.length > 0) throw new PatchError("layout", `the export lacks ${missing.slice(0, 10).join(", ")}`);
  const logo = files.find((file) => LOGOS.has(path.posix.basename(file.path)));
  if (logo !== undefined) throw new PatchError("logo", `the export ships ${logo.path}, which the bundle never contains`);
}

async function dependencyNotices(appRoot) {
  const lock = JSON.parse(await fs.readFile(path.join(appRoot, "package-lock.json"), "utf8"));
  const sections = [];
  for (const [key, info] of Object.entries(lock.packages).sort(([left], [right]) => (left < right ? -1 : 1))) {
    if (!key.startsWith("node_modules/") || info.dev === true || info.link === true) continue;
    const directory = path.join(appRoot, key);
    const names = (await fs.readdir(directory).catch(() => [])).filter((name) => LICENSE_FILE.test(name)).sort();
    if (names.length === 0) continue;
    const name = key.slice(key.lastIndexOf("node_modules/") + "node_modules/".length);
    const lines = [`==== ${name}@${info.version} (${info.license ?? "license not declared"}) ====`];
    for (const file of names) lines.push(`--- ${file} ---`, (await fs.readFile(path.join(directory, file), "utf8")).trimEnd());
    sections.push(lines.join("\n"));
  }
  if (sections.length === 0) throw new PatchError("notices", "no npm license file was found in node_modules");
  return sections;
}

async function legalComments(ui, files) {
  const comments = new Set();
  for (const file of files.filter((entry) => /^_next\/static\/.*\.(js|css)$/.test(entry.path))) {
    const text = await fs.readFile(path.join(ui, file.path), "utf8");
    for (const [comment] of text.matchAll(/\/\*[\s\S]*?\*\//g)) {
      if (comment.length < 100_000 && (comment.startsWith("/*!") || /@license|@preserve/.test(comment))) comments.add(comment);
    }
  }
  return [...comments].sort();
}

async function writeNotices({ ui, appRoot, license, files }) {
  const notices = path.join(ui, "notices");
  await fs.mkdir(notices, { recursive: true });
  await fs.copyFile(license, path.join(notices, "LiteLLM-LICENSE.txt"));
  const comments = await legalComments(ui, files);
  const text = [
    "Third-party notices for the npm packages and bundler legal comments in this dashboard export.",
    ...(await dependencyNotices(appRoot)),
    `==== bundler legal comments (${comments.length}) ====`,
    ...comments,
  ];
  await fs.writeFile(path.join(notices, "npm-dependencies.txt"), `${text.join("\n\n")}\n`);
}

async function normalizeModes(root) {
  await fs.chmod(root, 0o755);
  for (const entry of await fs.readdir(root, { recursive: true, withFileTypes: true })) {
    await fs.chmod(path.join(entry.parentPath, entry.name), entry.isDirectory() ? 0o755 : 0o644);
  }
}

export async function finalize({ exported, appRoot, license, bundle, inputs, versions }) {
  if (!HEX.test(inputs.key)) throw new PatchError("inputs", "the manifest inputs carry no cache key");
  const ui = path.join(bundle, "ui");
  await fs.rm(bundle, { recursive: true, force: true });
  await fs.mkdir(bundle, { recursive: true });
  await fs.cp(exported, ui, { recursive: true, verbatimSymlinks: true });
  const exportedFiles = await listTree(ui);
  await verifyLayout(exportedFiles, appRoot);
  await writeNotices({ ui, appRoot, license, files: exportedFiles });
  await fs.writeFile(path.join(ui, READY_MARKER), "");
  await normalizeModes(ui);
  const files = await listTree(ui);
  const manifest = { ...inputs, node: versions.node, npm: versions.npm, files: files.length, tree: treeDigest(files) };
  await fs.writeFile(path.join(bundle, "manifest.json"), `${JSON.stringify(manifest, null, 2)}\n`);
  return manifest;
}
