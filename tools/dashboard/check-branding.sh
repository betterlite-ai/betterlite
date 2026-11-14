#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
tools="${root}/tools/dashboard"
work="${root}/target/dashboard"
scratch="$(mktemp -d)"
trap 'rm -rf "${scratch}"' EXIT
source_args=()

fail() {
  printf 'check-branding: %s\n' "$1" >&2
  exit 1
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --source-repo)
      [[ $# -ge 2 ]] || fail '--source-repo needs a value'
      source_args=(--source-repo "$2")
      shift 2
      ;;
    *) fail "unknown argument $1" ;;
  esac
done

# the real export is what a user receives, so every check runs against it
bash "${tools}/build.sh" build "${source_args[@]}"

# preimages: the pinned app with the build patch only; postimages: the app the real patches produce
mkdir -p "${scratch}/patches"
cp "${root}/ui/dashboard/patches/10-build.patch" "${root}/ui/dashboard/patches/10-build.json" "${scratch}/patches/"
node "${tools}/source.mjs" prepare --out "${scratch}/prior" --cache "${work}/cache" --patches "${scratch}/patches" \
  "${source_args[@]}"
node "${tools}/source.mjs" prepare --out "${scratch}/patched" --cache "${work}/cache" "${source_args[@]}"

export BRANDING_ROOT="${root}" BRANDING_DIST="${work}/dist" BRANDING_PRIOR="${scratch}/prior" BRANDING_PATCHED="${scratch}/patched"
node --input-type=module << 'NODE'
import { createHash } from "node:crypto";
import fs from "node:fs/promises";
import path from "node:path";

const root = process.env.BRANDING_ROOT;
const dist = process.env.BRANDING_DIST;
const prior = process.env.BRANDING_PRIOR;
const patched = process.env.BRANDING_PATCHED;
const app = "ui/litellm-dashboard";
const patches = path.join(root, "ui/dashboard/patches");

const fail = (message) => {
  console.error(`check-branding: ${message}`);
  process.exit(1);
};

const digest = (bytes) => createHash("sha256").update(bytes).digest("hex");

const readOrNull = async (file) => {
  try {
    return await fs.readFile(file);
  } catch (error) {
    if (error.code === "ENOENT") return null;
    throw error;
  }
};

const listFiles = async (directory, base = "") => {
  const entries = await fs.readdir(directory, { withFileTypes: true });
  const files = [];
  for (const entry of entries) {
    const relative = base === "" ? entry.name : `${base}/${entry.name}`;
    if (entry.isDirectory()) files.push(...(await listFiles(path.join(directory, entry.name), relative)));
    else if (entry.isFile() || entry.isSymbolicLink()) files.push(relative);
  }
  return files;
};

const record = JSON.parse(await fs.readFile(path.join(patches, "20-branding.json"), "utf8"));
const patchText = await fs.readFile(path.join(patches, "20-branding.patch"), "utf8");
const patchPaths = [...patchText.matchAll(/^diff --git a\/(\S+) b\/(\S+)$/gm)].map((match) => match[2]).sort();
const recordPaths = record.targets.map((target) => target.path).sort();
if (JSON.stringify(patchPaths) !== JSON.stringify(recordPaths)) {
  fail("20-branding.patch and 20-branding.json name different targets");
}
for (const target of record.targets) {
  if (!target.path.startsWith(`${app}/`)) fail(`${target.path} is outside ${app}`);
  for (const [side, tree] of [["preimage", prior], ["postimage", patched]]) {
    const bytes = await readOrNull(path.join(tree, target.path));
    const actual = bytes === null ? null : digest(bytes);
    if (actual !== target[side]) fail(`${target.path} does not match its recorded ${side}`);
  }
}

const brandFiles = [
  "src/app/favicon.ico",
  "public/assets/logos/litellm.jpg",
  "public/assets/logos/litellm_logo.png",
  "public/assets/logos/litellm_logo_dark.png",
  "public/assets/logos/litellm_monogram.svg",
  "public/assets/logos/litellm_monogram_dark.svg",
];
const brandDigests = new Map();
for (const relative of brandFiles) {
  const pinned = await readOrNull(path.join(prior, app, relative));
  if (pinned === null) fail(`the pinned app has no ${relative}`);
  if ((await readOrNull(path.join(patched, app, relative))) !== null) fail(`the rebranded app still ships ${relative}`);
  brandDigests.set(digest(pinned), relative);
}

const ui = path.join(dist, "ui");
const exported = await listFiles(ui);
if (exported.length === 0) fail("the export is empty");
const needles = new Set([
  "betterlite Proxy Admin UI",
  "data-brand-wordmark",
  "notices/LiteLLM-LICENSE.txt",
  "LiteLLM license and notices (Berri AI, MIT)",
]);
const brandName = /^(favicon\.ico|litellm[^/]*\.(svg|png|jpe?g|gif|webp|avif|ico))$/i;
for (const relative of exported) {
  const bytes = await fs.readFile(path.join(ui, relative));
  const matched = brandDigests.get(digest(bytes));
  if (matched !== undefined) fail(`the export ships ${relative}, byte-identical to the pinned ${matched}`);
  const name = path.posix.basename(relative);
  if (brandName.test(name)) fail(`the export ships ${relative}, a LiteLLM brand asset name`);
  for (const needle of [...needles]) {
    if (bytes.includes(Buffer.from(needle))) needles.delete(needle);
  }
}
if (needles.size > 0) fail(`the export never contains ${[...needles].join(", ")}`);

for (const page of ["index.html", "login/index.html"]) {
  const html = await readOrNull(path.join(ui, page));
  if (html === null) fail(`the export has no ${page}`);
  for (const phrase of ["betterlite Dashboard", "betterlite Proxy Admin UI"]) {
    if (!html.includes(phrase)) fail(`${page} does not carry "${phrase}"`);
  }
  for (const phrase of ["LiteLLM Dashboard", "LiteLLM Proxy Admin UI"]) {
    if (html.includes(phrase)) fail(`${page} still carries "${phrase}"`);
  }
}

const license = await readOrNull(path.join(ui, "notices/LiteLLM-LICENSE.txt"));
const upstream = await readOrNull(path.join(patched, "LICENSE"));
if (license === null) fail("the export has no notices/LiteLLM-LICENSE.txt");
if (upstream === null) fail("the prepared app has no LICENSE");
if (!license.equals(upstream)) fail("notices/LiteLLM-LICENSE.txt is not the upstream Berri AI license");
const licenseText = license.toString("utf8");
for (const phrase of ["MIT License", "Copyright (c) 2023 Berri AI", "Permission is hereby granted", "THE SOFTWARE IS PROVIDED"]) {
  if (!licenseText.includes(phrase)) fail(`notices/LiteLLM-LICENSE.txt lacks "${phrase}"`);
}

const npm = await readOrNull(path.join(ui, "notices/npm-dependencies.txt"));
if (npm === null) fail("the export has no notices/npm-dependencies.txt");
const npmText = npm.toString("utf8");
if (!npmText.startsWith("Third-party notices for the npm packages and bundler legal comments in this dashboard export.")) {
  fail("notices/npm-dependencies.txt has no introduction");
}
for (const needle of ["==== react@", "==== next@", "==== bundler legal comments ("]) {
  if (!npmText.includes(needle)) fail(`notices/npm-dependencies.txt lacks a ${needle} section`);
}
const sections = npmText.split("\n").filter((line) => /^==== .+ ====$/.test(line)).length;
if (sections < 10) fail(`notices/npm-dependencies.txt carries only ${sections} license sections`);

console.log(`check-branding: ok, ${exported.length} export files checked against ${brandDigests.size} pinned brand assets`);
NODE
