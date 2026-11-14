import fs from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { applyPatches, sha256Hex } from "./patches.mjs";
import { obtainArchive, resolveSource } from "./source/cache.mjs";
import { SourceError } from "./source/errors.mjs";
import { runGit } from "./source/git.mjs";
import { extractArchive, isExcludedPath, isSelectedPath } from "./source/tar.mjs";

const SCRIPT_DIR = path.dirname(fileURLToPath(import.meta.url));
const DEFAULT_ROOT = path.resolve(SCRIPT_DIR, "../..");
const HASH = /^[0-9a-f]{64}$/;

export function checkPin(pin) {
  const fail = (message) => {
    throw new SourceError("pin-invalid", `source.pin.json: ${message}`);
  };
  const remote = pin.repository;
  if (typeof remote !== "string" || !(remote.startsWith("https://") || remote.startsWith("file://") || path.isAbsolute(remote))) {
    fail("repository must be an https URL, a file URL or an absolute path");
  }
  if (typeof pin.commit !== "string" || !/^[0-9a-f]{40}$/.test(pin.commit)) fail("commit must be a full object id");
  if (typeof pin.dashboardPath !== "string" || pin.dashboardPath.length === 0) fail("dashboardPath is missing");
  if (typeof pin.dashboardTree !== "string" || !/^[0-9a-f]{40}$/.test(pin.dashboardTree)) fail("dashboardTree must be a tree id");
  if (typeof pin.archiveSha256 !== "string" || !HASH.test(pin.archiveSha256)) fail("archiveSha256 must be a SHA-256");
  if (!Array.isArray(pin.archivePaths) || pin.archivePaths.length === 0) fail("archivePaths must not be empty");
  if (!Array.isArray(pin.excludedPrefixes)) fail("excludedPrefixes must be an array");
  if (!Array.isArray(pin.protectedFiles) || pin.protectedFiles.length === 0) fail("protectedFiles must not be empty");
  for (const file of pin.protectedFiles) {
    if (typeof file.path !== "string" || typeof file.sha256 !== "string" || !HASH.test(file.sha256)) {
      fail("every protected file needs a path and a SHA-256");
    }
  }
  return pin;
}

export async function loadPin(pinPath) {
  let bytes;
  try {
    bytes = await fs.readFile(pinPath);
  } catch (error) {
    if (error.code === "ENOENT") throw new SourceError("pin-missing", `${pinPath} does not exist`);
    throw error;
  }
  let pin;
  try {
    pin = JSON.parse(bytes.toString("utf8"));
  } catch (error) {
    throw new SourceError("pin-invalid", `source.pin.json is not valid JSON: ${error.message}`);
  }
  return checkPin(pin);
}

async function verifyProtected(root, pin) {
  for (const file of pin.protectedFiles) {
    let bytes;
    try {
      bytes = await fs.readFile(path.join(root, file.path));
    } catch (error) {
      if (error.code === "ENOENT") throw new SourceError("protected-file", `${file.path} is missing from the prepared tree`);
      throw error;
    }
    const actual = sha256Hex(bytes);
    if (actual !== file.sha256) {
      throw new SourceError("protected-file", `${file.path} has SHA-256 ${actual}, upstream is ${file.sha256}`);
    }
  }
}

async function publish(staging, outDir) {
  const trash = `${outDir}.old-${process.pid}-${Date.now()}`;
  let moved = false;
  try {
    await fs.rename(outDir, trash);
    moved = true;
  } catch (error) {
    if (error.code !== "ENOENT") throw error;
  }
  try {
    await fs.rename(staging, outDir);
  } catch (error) {
    if (moved) await fs.rename(trash, outDir).catch(() => undefined);
    throw error;
  }
  if (moved) await fs.rm(trash, { recursive: true, force: true }).catch(() => undefined);
}

export function resolveSettings(options) {
  const repoRoot = path.resolve(options.repoRoot ?? DEFAULT_ROOT);
  return {
    repoRoot,
    pinPath: path.resolve(options.pinPath ?? path.join(repoRoot, "tools/dashboard/source.pin.json")),
    cacheDir: path.resolve(options.cacheDir ?? path.join(repoRoot, "target/dashboard/cache")),
    outDir: path.resolve(options.outDir ?? path.join(repoRoot, "target/dashboard/source")),
    patchesDir: path.resolve(options.patchesDir ?? path.join(repoRoot, "ui/dashboard/patches")),
    verifyDir: options.verifyDir ? path.resolve(options.verifyDir) : null,
    sourceRepo: options.sourceRepo ? path.resolve(options.sourceRepo) : null,
    commandTimeoutMs: options.timeoutMs ?? 60_000,
    fetchTimeoutMs: options.timeoutMs ?? 600_000,
    archiveTimeoutMs: options.timeoutMs ?? 120_000,
  };
}

export async function prepare(options = {}) {
  const settings = resolveSettings(options);
  const pin = options.pin ?? await loadPin(settings.pinPath);
  checkPin(pin);
  const source = await resolveSource(settings, pin);
  const parent = path.dirname(settings.outDir);
  await fs.mkdir(parent, { recursive: true });
  const staging = await fs.mkdtemp(path.join(parent, ".source-"));
  let archive = null;
  let published = false;
  try {
    archive = await obtainArchive(settings, pin, source);
    const bytes = await fs.readFile(archive.file);
    const entries = await extractArchive(bytes, staging, pin);
    await verifyProtected(staging, pin);
    const patches = await applyPatches({
      root: staging,
      patchesDir: settings.patchesDir,
      isAllowedPath: (relative) => isSelectedPath(relative, pin) && !isExcludedPath(relative, pin),
      isProtectedPath: (relative) => pin.protectedFiles.some((file) => file.path === relative),
      runGit,
      timeoutMs: settings.commandTimeoutMs,
    });
    await verifyProtected(staging, pin);
    await fs.chmod(staging, 0o775);
    await publish(staging, settings.outDir);
    published = true;
    return { commit: pin.commit, archiveSha256: pin.archiveSha256, outDir: settings.outDir, entries, patches };
  } finally {
    if (archive !== null && archive.temporary) await fs.rm(archive.file, { force: true }).catch(() => undefined);
    if (!published) await fs.rm(staging, { recursive: true, force: true }).catch(() => undefined);
  }
}

if (process.argv[1] !== undefined && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  import("./source/cli.mjs")
    .then(({ main }) => main())
    .catch((error) => {
      console.error(`error: ${error.message}`);
      process.exit(1);
    });
}
