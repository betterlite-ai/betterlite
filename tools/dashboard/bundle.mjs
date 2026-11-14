import { execFileSync } from "node:child_process";
import fs from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { parseArgs } from "node:util";

import { PatchError, sha256Hex } from "./patches.mjs";

const APP = "ui/litellm-dashboard";
const READY_MARKER = ".litellm_ui_ready";
const LOGOS = new Set(["logo.png", "logo_dark.png", "logo_monogram.png", "logo_monogram_dark.png", "cached_logo.jpg"]);
const LICENSE_FILE = /^(licen[cs]e|notice|copying|copyright)(\.|$)/i;
const BUILDER = /^ARG UI_BUILD_IMAGE=(\S+@sha256:[0-9a-f]{64})$/m;
const HEX = /^[0-9a-f]{64}$/;
const ROUTE_GROUP = /^\([^.][^)]*\)$/;

export async function listTree(root) {
  const files = [];
  const pending = [""];
  while (pending.length > 0) {
    const relative = pending.pop();
    for (const entry of await fs.readdir(path.join(root, relative), { withFileTypes: true })) {
      const child = relative === "" ? entry.name : `${relative}/${entry.name}`;
      if (entry.isDirectory()) {
        pending.push(child);
      } else if (entry.isFile()) {
        const full = path.join(root, child);
        const { mode } = await fs.stat(full);
        files.push({ path: child, mode: mode & 0o777, sha256: sha256Hex(await fs.readFile(full)) });
      } else {
        throw new PatchError("tree-entry", `${child} is neither a regular file nor a directory`);
      }
    }
  }
  return files.sort((left, right) => (left.path < right.path ? -1 : 1));
}

export function treeDigest(files) {
  return sha256Hex(files.map((file) => `${file.path}\0${file.mode.toString(8)}\0${file.sha256}\n`).join(""));
}

export function checkApiBase(value) {
  if (value === "") return;
  let url;
  try {
    url = new URL(value);
  } catch {
    throw new PatchError("api-base", `--api-base ${JSON.stringify(value)} is not an absolute URL`);
  }
  if (!["http:", "https:"].includes(url.protocol) || url.username !== "" || url.password !== "" || /\s/.test(value)) {
    throw new PatchError("api-base", "--api-base must be an http(s) URL without credentials or spaces");
  }
}

export function cacheFlags(env) {
  const mode = env.DASHBOARD_BUILDKIT_CACHE ?? "";
  if (mode === "") return [];
  if (mode !== "read" && mode !== "read-write") {
    throw new PatchError("cache-mode", `DASHBOARD_BUILDKIT_CACHE is ${JSON.stringify(mode)}, expected read or read-write`);
  }
  if (!env.ACTIONS_RESULTS_URL || !env.ACTIONS_RUNTIME_TOKEN) return [];
  const flags = ["--cache-from", "type=gha,scope=dashboard"];
  if (mode === "read-write") flags.push("--cache-to", "type=gha,scope=dashboard,mode=max");
  return flags;
}

async function readPatchFiles(patchesDir) {
  let names;
  try {
    names = (await fs.readdir(patchesDir, { withFileTypes: true }))
      .filter((entry) => entry.isFile() && entry.name.endsWith(".patch"))
      .map((entry) => entry.name)
      .sort();
  } catch (error) {
    if (error.code === "ENOENT") return [];
    throw error;
  }
  const patches = [];
  for (const name of names) {
    const recordName = `${name.slice(0, -".patch".length)}.json`;
    try {
      patches.push({
        name,
        patch: await fs.readFile(path.join(patchesDir, name)),
        record: await fs.readFile(path.join(patchesDir, recordName)),
      });
    } catch (error) {
      if (error.code === "ENOENT") throw new PatchError("patch-record", `${name} has no adjacent ${recordName}`);
      throw error;
    }
  }
  return patches;
}

export async function bundleInputs({ repoRoot, apiBase = "" }) {
  checkApiBase(apiBase);
  const read = (relative) => fs.readFile(path.join(repoRoot, relative));
  const pin = JSON.parse(await read("tools/dashboard/source.pin.json"));
  const pinned = (name) => {
    const file = pin.protectedFiles.find((entry) => entry.path === `${APP}/${name}`);
    if (file === undefined) throw new PatchError("pin", `source.pin.json does not pin ${name}`);
    return file.sha256;
  };
  const patches = (await readPatchFiles(path.join(repoRoot, "ui/dashboard/patches"))).map((file) => ({
    name: file.name,
    patch: sha256Hex(file.patch),
    record: sha256Hex(file.record),
  }));
  const dockerfile = await read("packaging/ui/Dockerfile");
  const builder = BUILDER.exec(dockerfile.toString("utf8"))?.[1];
  if (builder === undefined) throw new PatchError("builder", "packaging/ui/Dockerfile has no digest-pinned UI_BUILD_IMAGE");
  const source = { commit: pin.commit, archiveSha256: pin.archiveSha256 };
  const buildId = sha256Hex(JSON.stringify({ source: source.archiveSha256, patches, apiBase })).slice(0, 20);
  const key = sha256Hex(
    JSON.stringify({
      source: source.archiveSha256,
      patches,
      lock: pinned("package-lock.json"),
      npmrc: pinned(".npmrc"),
      builder,
      dockerfile: sha256Hex(dockerfile),
      buildScript: sha256Hex(await read("tools/dashboard/build.sh")),
      bundleScript: sha256Hex(await read("tools/dashboard/bundle.mjs")),
      apiBase,
    }),
  );
  return { key, buildId, apiBase, builder, source, patches: patches.map((patch) => patch.name) };
}

async function patchedSources(patchesDir) {
  const sources = new Set();
  for (const file of await readPatchFiles(patchesDir)) {
    for (const target of JSON.parse(file.record).targets) {
      const relative = target.path.slice(`${APP}/`.length);
      if (target.postimage !== null && /\.tsx?$/.test(relative)) sources.add(relative);
    }
  }
  return [...sources].sort();
}

async function extraTests(testsDir, appRoot) {
  let text;
  try {
    text = await fs.readFile(path.join(testsDir, "upstream-tests.json"), "utf8");
  } catch (error) {
    if (error.code === "ENOENT") return [];
    throw error;
  }
  const list = JSON.parse(text);
  if (!Array.isArray(list) || list.some((entry) => typeof entry !== "string")) {
    throw new PatchError("upstream-tests", "upstream-tests.json must be an array of paths");
  }
  const appReal = await fs.realpath(appRoot);
  for (const entry of list) {
    const inside = entry !== "" && !path.posix.isAbsolute(entry) && path.posix.normalize(entry) === entry && !entry.startsWith("../");
    if (!inside || entry.split("/").includes("node_modules")) {
      throw new PatchError("upstream-tests", `${entry} is outside the prepared application`);
    }
    if (!/\.test\.tsx?$/.test(entry)) throw new PatchError("upstream-tests", `${entry} is not a Vitest file`);
    const real = await fs.realpath(path.join(appRoot, entry)).catch(() => null);
    if (real === null || !real.startsWith(`${appReal}${path.sep}`) || !(await fs.stat(real)).isFile()) {
      throw new PatchError("upstream-tests", `${entry} has no tests in the prepared application`);
    }
  }
  return list;
}

export async function writePlan({ testsDir, appRoot, repoRoot, out }) {
  const stat = await fs.stat(testsDir).catch(() => null);
  if (stat === null || !stat.isDirectory()) throw new PatchError("tests-dir", `${testsDir} is not a directory`);
  await listTree(testsDir);
  const extras = await extraTests(testsDir, appRoot);
  const patched = await patchedSources(path.join(repoRoot, "ui/dashboard/patches"));
  await fs.rm(out, { recursive: true, force: true });
  await fs.mkdir(out, { recursive: true });
  await fs.cp(testsDir, path.join(out, "tests"), { recursive: true });
  const plan = { tests: ["tests/betterlite", ...extras], files: [...patched, "tests/betterlite/**/*.{ts,tsx}"] };
  await fs.writeFile(path.join(out, "plan.json"), `${JSON.stringify(plan, null, 2)}\n`);
  return plan;
}

export function runChecks({ appRoot, plan }) {
  const bin = (name) => path.join(appRoot, "node_modules/.bin", name);
  const run = (name, args) => execFileSync(bin(name), args, { cwd: appRoot, stdio: "inherit" });
  for (const target of plan.tests) run("vitest", ["run", target]);
  run("eslint", plan.files);
  run("prettier", ["--check", ...plan.files]);
}

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

export async function main(argv, env) {
  const { positionals, values } = parseArgs({
    args: argv,
    allowPositionals: true,
    options: {
      "api-base": { type: "string", default: "" },
      "repo-root": { type: "string", default: path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..") },
      tests: { type: "string" },
      app: { type: "string", default: "/work/dashboard" },
      out: { type: "string" },
      exported: { type: "string" },
      license: { type: "string" },
      bundle: { type: "string" },
      plan: { type: "string" },
    },
  });
  const repoRoot = path.resolve(values["repo-root"]);
  const appRoot = path.resolve(values.app);
  switch (positionals[0]) {
    case "key":
    case "build-id":
    case "inputs": {
      const inputs = await bundleInputs({ repoRoot, apiBase: values["api-base"] });
      const printed = { key: inputs.key, "build-id": inputs.buildId, inputs: JSON.stringify(inputs) };
      process.stdout.write(`${printed[positionals[0]]}\n`);
      break;
    }
    case "cache-flags":
      process.stdout.write(`${cacheFlags(env).join(" ")}\n`);
      break;
    case "plan":
      await writePlan({ testsDir: path.resolve(values.tests), appRoot, repoRoot, out: path.resolve(values.out) });
      break;
    case "finalize": {
      const npm = execFileSync("npm", ["--version"], { encoding: "utf8" }).trim();
      const manifest = await finalize({
        exported: path.resolve(values.exported),
        appRoot,
        license: path.resolve(values.license),
        bundle: path.resolve(values.bundle),
        inputs: JSON.parse(env.MANIFEST_INPUTS ?? "{}"),
        versions: { node: process.version, npm },
      });
      process.stdout.write(`bundle ${manifest.key}: ${manifest.files} files, node ${manifest.node}, npm ${manifest.npm}\n`);
      break;
    }
    case "check": {
      const planFile = path.resolve(values.plan);
      runChecks({ appRoot, plan: JSON.parse(await fs.readFile(planFile, "utf8")) });
      break;
    }
    default:
      throw new PatchError("usage", "usage: bundle.mjs key|build-id|inputs|cache-flags|plan|finalize|check [options]");
  }
}

if (process.argv[1] !== undefined && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main(process.argv.slice(2), process.env).catch((error) => {
    console.error(`error: ${error.message}`);
    process.exit(1);
  });
}
