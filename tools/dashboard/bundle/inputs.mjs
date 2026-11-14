import fs from "node:fs/promises";
import path from "node:path";

import { PatchError, readPatchFiles, sha256Hex } from "../patches.mjs";

export const APP = "ui/litellm-dashboard";
const BUILDER = /^ARG UI_BUILD_IMAGE=(\S+@sha256:[0-9a-f]{64})$/m;
const BUNDLE_DIR = "tools/dashboard/bundle";

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

async function moduleHashes(repoRoot) {
  const files = await listTree(path.join(repoRoot, BUNDLE_DIR));
  return Object.fromEntries(files.map((file) => [`${BUNDLE_DIR}/${file.path}`, file.sha256]));
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
      patchScript: sha256Hex(await read("tools/dashboard/patches.mjs")),
      modules: await moduleHashes(repoRoot),
      apiBase,
    }),
  );
  return { key, buildId, apiBase, builder, source, patches: patches.map((patch) => patch.name) };
}
