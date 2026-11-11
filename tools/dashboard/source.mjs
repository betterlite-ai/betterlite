import { spawn } from "node:child_process";
import { createHash } from "node:crypto";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { applyPatches } from "./patches.mjs";

const SCRIPT_DIR = path.dirname(fileURLToPath(import.meta.url));
const DEFAULT_ROOT = path.resolve(SCRIPT_DIR, "../..");
const MAX_SMALL_OUTPUT = 4 * 1024 * 1024;
const MAX_ARCHIVE_OUTPUT = 512 * 1024 * 1024;
const HASH = /^[0-9a-f]{64}$/;
const CHILDREN = new Set();
const DROPPED_ENVIRONMENT = [
  "GIT_DIR",
  "GIT_WORK_TREE",
  "GIT_INDEX_FILE",
  "GIT_OBJECT_DIRECTORY",
  "GIT_ALTERNATE_OBJECT_DIRECTORIES",
  "GIT_NAMESPACE",
  "GIT_COMMON_DIR",
  "GIT_CEILING_DIRECTORIES",
  "GIT_CONFIG_COUNT",
  "GIT_CONFIG_PARAMETERS",
  "GIT_ATTR_SOURCE",
];

export class SourceError extends Error {
  constructor(code, message) {
    super(message);
    this.name = "SourceError";
    this.code = code;
  }
}

function sha256(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

function isolatedEnvironment(extra) {
  const environment = { ...process.env };
  for (const name of Object.keys(environment)) {
    if (DROPPED_ENVIRONMENT.includes(name) || name.startsWith("GIT_CONFIG_KEY_") || name.startsWith("GIT_CONFIG_VALUE_")) {
      delete environment[name];
    }
  }
  return {
    ...environment,
    GIT_CONFIG_GLOBAL: os.devNull,
    GIT_CONFIG_SYSTEM: os.devNull,
    GIT_CONFIG_NOSYSTEM: "1",
    GIT_ATTR_NOSYSTEM: "1",
    GIT_TERMINAL_PROMPT: "0",
    ...extra,
  };
}

function killGroup(child) {
  try {
    process.kill(-child.pid, "SIGKILL");
  } catch (error) {
    if (error.code !== "ESRCH") throw error;
  }
}

function spawnGit(args, options) {
  return new Promise((resolve, reject) => {
    const child = spawn("git", args, {
      cwd: options.cwd,
      env: isolatedEnvironment(options.env),
      detached: true,
      stdio: ["ignore", "pipe", "pipe"],
    });
    CHILDREN.add(child);
    const chunks = [];
    let captured = 0;
    let stderr = "";
    let timedOut = false;
    const limit = options.maxStdoutBytes ?? MAX_SMALL_OUTPUT;
    const timer = setTimeout(() => {
      timedOut = true;
      killGroup(child);
    }, options.timeoutMs);
    const finish = (callback) => {
      clearTimeout(timer);
      CHILDREN.delete(child);
      callback();
    };
    child.stdout.on("data", (chunk) => {
      captured += chunk.length;
      if (captured > limit) {
        killGroup(child);
        finish(() => reject(new SourceError("output-limit", `git ${args[0]} wrote more than ${limit} bytes`)));
        return;
      }
      chunks.push(chunk);
    });
    child.stderr.setEncoding("utf8");
    child.stderr.on("data", (chunk) => {
      if (stderr.length < 8192) stderr += chunk;
    });
    child.on("error", (error) => {
      finish(() => reject(new SourceError("git-spawn", `could not run git: ${error.message}`)));
    });
    child.on("close", (status) => {
      finish(() => {
        if (timedOut) {
          reject(new SourceError("timeout", `git ${args[0]} did not finish within ${options.timeoutMs} ms`));
          return;
        }
        resolve({ status, stdout: Buffer.concat(chunks), stderr: stderr.trim() });
      });
    });
  });
}

async function runGit(args, options = {}) {
  const result = await spawnGit(args, {
    cwd: options.cwd ?? process.cwd(),
    timeoutMs: options.timeoutMs ?? 60_000,
    env: options.ceiling ? { GIT_CEILING_DIRECTORIES: options.ceiling } : undefined,
    maxStdoutBytes: options.maxStdoutBytes,
  });
  if (result.status !== 0 && options.allowFailure !== true) {
    throw new SourceError(options.code ?? "git", `git ${args[0]} failed: ${result.stderr || `exit ${result.status}`}`);
  }
  return result;
}

function text(result) {
  return result.stdout.toString("utf8").trim();
}

function checkPin(pin) {
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

async function verifyCommit(repoDir, pin, timeoutMs) {
  const type = await runGit(["-C", repoDir, "cat-file", "-t", pin.commit], { timeoutMs, allowFailure: true });
  if (type.status !== 0 || text(type) !== "commit") {
    throw new SourceError("source-commit", `${repoDir} does not contain the pinned commit ${pin.commit}`);
  }
  const peeled = await runGit(["-C", repoDir, "rev-parse", "--verify", "--quiet", `${pin.commit}^{commit}`], { timeoutMs, allowFailure: true });
  if (peeled.status !== 0 || text(peeled) !== pin.commit) {
    throw new SourceError("source-commit", `${pin.commit} is not the pinned commit object; tags and other object kinds are refused`);
  }
  const tree = await runGit(["-C", repoDir, "rev-parse", "--verify", "--quiet", `${pin.commit}:${pin.dashboardPath}`], { timeoutMs, allowFailure: true });
  if (tree.status !== 0 || text(tree) !== pin.dashboardTree) {
    throw new SourceError("dashboard-tree", `${pin.dashboardPath} at ${pin.commit} is not tree ${pin.dashboardTree}`);
  }
}

async function ensureCacheRepo(settings, pin) {
  const repoDir = path.join(settings.cacheDir, "repo.git");
  let exists = true;
  try {
    const stat = await fs.stat(repoDir);
    exists = stat.isDirectory();
  } catch (error) {
    if (error.code !== "ENOENT") throw error;
    exists = false;
  }
  if (exists) {
    const gitDir = await runGit(["-C", repoDir, "rev-parse", "--git-dir"], { timeoutMs: settings.commandTimeoutMs, allowFailure: true });
    if (gitDir.status !== 0) {
      throw new SourceError("cache-corrupt", `${repoDir} is not a usable git repository; delete ${settings.cacheDir} to refetch`);
    }
    const fsck = await runGit(["-C", repoDir, "fsck", "--no-progress"], { timeoutMs: settings.archiveTimeoutMs, allowFailure: true });
    if (fsck.status !== 0) {
      throw new SourceError("cache-corrupt", `the git cache in ${repoDir} fails verification (${fsck.stderr}); delete ${settings.cacheDir} to refetch`);
    }
    const present = await runGit(["-C", repoDir, "cat-file", "-e", `${pin.commit}^{commit}`], { timeoutMs: settings.commandTimeoutMs, allowFailure: true });
    if (present.status === 0) {
      await verifyCommit(repoDir, pin, settings.commandTimeoutMs);
      return repoDir;
    }
  } else {
    await fs.mkdir(settings.cacheDir, { recursive: true });
    await runGit(["init", "--bare", "--quiet", repoDir], { timeoutMs: settings.commandTimeoutMs });
  }
  await runGit(
    ["-C", repoDir, "fetch", "--no-tags", "--depth=1", "--no-recurse-submodules", "--quiet", pin.repository, pin.commit],
    { timeoutMs: settings.fetchTimeoutMs, code: "fetch", maxStdoutBytes: MAX_SMALL_OUTPUT },
  );
  await verifyCommit(repoDir, pin, settings.commandTimeoutMs);
  return repoDir;
}

async function resolveSource(settings, pin) {
  if (settings.sourceRepo !== null) {
    const repoDir = settings.sourceRepo;
    const inside = await runGit(["-C", repoDir, "rev-parse", "--git-dir"], { timeoutMs: settings.commandTimeoutMs, allowFailure: true });
    if (inside.status !== 0) {
      throw new SourceError("source-repo", `${repoDir} is not a git repository`);
    }
    await verifyCommit(repoDir, pin, settings.commandTimeoutMs);
    return { repoDir, cached: false };
  }
  return { repoDir: await ensureCacheRepo(settings, pin), cached: true };
}

async function archiveBytes(repoDir, pin, timeoutMs) {
  const result = await runGit(
    [
      "-C", repoDir,
      "-c", "tar.umask=0002",
      "-c", `core.attributesFile=${os.devNull}`,
      "archive", "--format=tar", pin.commit,
      ...pin.archivePaths,
    ],
    { timeoutMs, code: "archive", maxStdoutBytes: MAX_ARCHIVE_OUTPUT },
  );
  return result.stdout;
}

async function obtainArchive(settings, pin, source) {
  const cacheArchive = path.join(settings.cacheDir, "archive.tar");
  if (source.cached) {
    let cached = null;
    try {
      cached = await fs.readFile(cacheArchive);
    } catch (error) {
      if (error.code !== "ENOENT") throw error;
    }
    if (cached !== null) {
      const actual = sha256(cached);
      if (actual !== pin.archiveSha256) {
        throw new SourceError("cache-corrupt", `${cacheArchive} has SHA-256 ${actual}, the pin expects ${pin.archiveSha256}; delete ${settings.cacheDir} to refetch`);
      }
      return { file: cacheArchive, temporary: false };
    }
  }
  const bytes = await archiveBytes(source.repoDir, pin, settings.archiveTimeoutMs);
  const actual = sha256(bytes);
  if (actual !== pin.archiveSha256) {
    throw new SourceError("archive-mismatch", `git archive produced SHA-256 ${actual}, the pin expects ${pin.archiveSha256}`);
  }
  if (!source.cached) {
    const temporary = path.join(path.dirname(settings.outDir), `.archive-${process.pid}-${Date.now()}.tar`);
    await fs.writeFile(temporary, bytes);
    return { file: temporary, temporary: true };
  }
  await fs.mkdir(settings.cacheDir, { recursive: true });
  const temporary = `${cacheArchive}.tmp-${process.pid}-${Date.now()}`;
  await fs.writeFile(temporary, bytes);
  await fs.rename(temporary, cacheArchive);
  return { file: cacheArchive, temporary: false };
}

function readString(buffer, start, length) {
  let end = start;
  const limit = Math.min(buffer.length, start + length);
  while (end < limit && buffer[end] !== 0) end += 1;
  return buffer.toString("utf8", start, end);
}

function readOctal(buffer, start, length, field) {
  const value = readString(buffer, start, length).trim();
  if (value === "") return 0;
  const parsed = Number.parseInt(value, 8);
  if (!Number.isSafeInteger(parsed) || parsed < 0) {
    throw new SourceError("tar-invalid", `tar header field ${field} is not octal`);
  }
  return parsed;
}

function verifyChecksum(header) {
  const expected = readOctal(header, 148, 8, "checksum");
  let sum = 0;
  for (let index = 0; index < header.length; index += 1) {
    sum += index >= 148 && index < 156 ? 0x20 : header[index];
  }
  if (sum !== expected) {
    throw new SourceError("tar-invalid", "a tar header checksum does not match its bytes");
  }
}

function paxRecords(payload) {
  const values = new Map();
  let offset = 0;
  while (offset < payload.length) {
    const space = payload.indexOf(0x20, offset);
    if (space < 0) break;
    const length = Number.parseInt(payload.toString("ascii", offset, space), 10);
    if (!Number.isSafeInteger(length) || length <= 0 || offset + length > payload.length) {
      throw new SourceError("tar-invalid", "a pax header record has an invalid length");
    }
    const record = payload.toString("utf8", space + 1, offset + length).replace(/\n$/, "");
    const equals = record.indexOf("=");
    if (equals > 0) values.set(record.slice(0, equals), record.slice(equals + 1));
    offset += length;
  }
  return values;
}

function relativeEntry(name, directory) {
  const trimmed = directory ? name.replace(/\/+$/, "") : name;
  if (trimmed.length === 0) return null;
  if (trimmed.includes("\0") || trimmed.startsWith("/")) {
    throw new SourceError("unsafe-path", `tar entry ${JSON.stringify(name)} is absolute or holds a NUL`);
  }
  const segments = trimmed.split("/");
  if (segments.some((segment) => segment === "" || segment === "." || segment === "..")) {
    throw new SourceError("unsafe-path", `tar entry ${JSON.stringify(name)} escapes or is not normalized`);
  }
  return segments.join("/");
}

export function isSelectedPath(relative, pin) {
  return pin.archivePaths.some((root) => relative === root || relative.startsWith(`${root}/`) || root.startsWith(`${relative}/`));
}

export function isExcludedPath(relative, pin) {
  if (relative.split("/").some((segment) => segment === "enterprise" || segment.endsWith("_enterprise"))) return true;
  return pin.excludedPrefixes.some((prefix) => relative === prefix || relative.startsWith(`${prefix}/`));
}

function checkLink(root, absolute, target) {
  if (target.length === 0 || target.includes("\0") || path.isAbsolute(target)) {
    throw new SourceError("unsafe-link", `tar symlink ${absolute} points outside the export`);
  }
  const resolved = path.resolve(path.dirname(absolute), target);
  if (resolved !== root && !resolved.startsWith(`${root}${path.sep}`)) {
    throw new SourceError("unsafe-link", `tar symlink ${absolute} points outside the export`);
  }
}

async function writeEntry(root, relative, mode, payload) {
  const absolute = path.join(root, relative);
  await fs.mkdir(path.dirname(absolute), { recursive: true });
  await fs.writeFile(absolute, payload);
  await fs.chmod(absolute, mode & 0o777);
}

async function writeDirectory(root, relative, mode, directoryModes) {
  const absolute = path.join(root, relative);
  await fs.mkdir(absolute, { recursive: true });
  directoryModes.push([absolute, mode & 0o777]);
}

export async function extractArchive(bytes, destDir, pin) {
  const root = path.resolve(destDir);
  await fs.mkdir(root, { recursive: true });
  const directoryModes = [];
  let offset = 0;
  let pending = null;
  let written = 0;
  while (offset + 512 <= bytes.length) {
    const header = bytes.subarray(offset, offset + 512);
    if (header.every((byte) => byte === 0)) break;
    verifyChecksum(header);
    let name = readString(header, 0, 100);
    if (readString(header, 257, 5) === "ustar") {
      const prefix = readString(header, 345, 155);
      if (prefix !== "") name = `${prefix}/${name}`;
    }
    let mode = readOctal(header, 100, 8, "mode");
    const bodyStart = offset + 512;
    const headerSize = readOctal(header, 124, 12, "size");
    const type = String.fromCharCode(header[156] === 0 ? 0x30 : header[156]);
    let linkname = readString(header, 157, 100);
    if (type === "x" || type === "g") {
      if (bodyStart + headerSize > bytes.length) {
        throw new SourceError("tar-invalid", `a pax header runs past the end of the archive`);
      }
      pending = type === "x" ? paxRecords(bytes.subarray(bodyStart, bodyStart + headerSize)) : null;
      offset = bodyStart + Math.ceil(headerSize / 512) * 512;
      continue;
    }
    let size = headerSize;
    if (pending !== null) {
      if (pending.has("path")) name = pending.get("path");
      if (pending.has("linkpath")) linkname = pending.get("linkpath");
      if (pending.has("size")) {
        const overridden = Number.parseInt(pending.get("size"), 10);
        if (!Number.isSafeInteger(overridden) || overridden < 0) {
          throw new SourceError("tar-invalid", `${name}: its pax size is not a number`);
        }
        size = overridden;
      }
      pending = null;
    }
    if (bodyStart + size > bytes.length) {
      throw new SourceError("tar-invalid", `${name} runs past the end of the archive`);
    }
    const payload = bytes.subarray(bodyStart, bodyStart + size);
    offset = bodyStart + Math.ceil(size / 512) * 512;
    if (type === "5") {
      const relative = relativeEntry(name, true);
      if (relative === null) continue;
      if (isExcludedPath(relative, pin)) continue;
      if (!isSelectedPath(relative, pin)) {
        throw new SourceError("unselected-path", `tar entry ${relative} is outside the selected export`);
      }
      await writeDirectory(root, relative, mode, directoryModes);
      written += 1;
      continue;
    }
    const relative = relativeEntry(name, false);
    if (relative === null) {
      throw new SourceError("tar-invalid", "a tar entry has an empty name");
    }
    if (isExcludedPath(relative, pin)) continue;
    if (!isSelectedPath(relative, pin)) {
      throw new SourceError("unselected-path", `tar entry ${relative} is outside the selected export`);
    }
    if (type === "0") {
      await writeEntry(root, relative, mode, payload);
      written += 1;
    } else if (type === "2") {
      const absolute = path.join(root, relative);
      checkLink(root, absolute, linkname);
      await fs.mkdir(path.dirname(absolute), { recursive: true });
      await fs.symlink(linkname, absolute);
      written += 1;
    } else {
      throw new SourceError("unsupported-entry", `tar entry ${relative} has unsupported type ${type}`);
    }
  }
  for (const [absolute, mode] of directoryModes) {
    await fs.chmod(absolute, mode);
  }
  return written;
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
    const actual = sha256(bytes);
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

function resolveSettings(options) {
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

async function snapshot(root) {
  const entries = [];
  const walk = async (directory, relative) => {
    const names = await fs.readdir(directory, { withFileTypes: true });
    names.sort((first, second) => (first.name < second.name ? -1 : first.name > second.name ? 1 : 0));
    for (const name of names) {
      const absolute = path.join(directory, name.name);
      const child = relative === "" ? name.name : `${relative}/${name.name}`;
      const stat = await fs.lstat(absolute);
      if (stat.isDirectory()) {
        entries.push({ path: child, mode: stat.mode & 0o777, sha256: null });
        await walk(absolute, child);
      } else if (stat.isSymbolicLink()) {
        entries.push({ path: child, mode: stat.mode & 0o777, sha256: sha256(await fs.readlink(absolute)) });
      } else if (stat.isFile()) {
        entries.push({ path: child, mode: stat.mode & 0o777, sha256: sha256(await fs.readFile(absolute)) });
      } else {
        throw new SourceError("unexpected-entry", `${absolute} is not a file, directory or symlink`);
      }
    }
  };
  await walk(root, "");
  return entries;
}

export async function compareTrees(first, second) {
  const left = await snapshot(first);
  const right = await snapshot(second);
  for (let index = 0; index < Math.max(left.length, right.length); index += 1) {
    const one = left[index];
    const two = right[index];
    if (one === undefined || two === undefined || one.path !== two.path || one.mode !== two.mode || one.sha256 !== two.sha256) {
      throw new SourceError("reproducibility", `the two preparations differ at ${one?.path ?? two?.path}: ${JSON.stringify(one)} and ${JSON.stringify(two)}`);
    }
  }
  return left.length;
}

export async function verifyRepeat(options = {}) {
  const settings = resolveSettings(options);
  const pin = options.pin ?? await loadPin(settings.pinPath);
  checkPin(pin);
  const verifyDir = settings.verifyDir ?? path.join(path.dirname(settings.cacheDir), "verify");
  await fs.mkdir(verifyDir, { recursive: true });
  const first = await fs.mkdtemp(path.join(verifyDir, "first-"));
  const second = await fs.mkdtemp(path.join(verifyDir, "second-"));
  try {
    const one = await prepare({ ...options, pin, outDir: first });
    await prepare({ ...options, pin, outDir: second });
    const entries = await compareTrees(first, second);
    return { commit: pin.commit, archiveSha256: one.archiveSha256, entries, patches: one.patches };
  } finally {
    await fs.rm(first, { recursive: true, force: true }).catch(() => undefined);
    await fs.rm(second, { recursive: true, force: true }).catch(() => undefined);
  }
}

function optionValue(argv, index, name) {
  const value = argv[index + 1];
  if (value === undefined || value.startsWith("--")) {
    throw new SourceError("usage", `${name} needs a value`);
  }
  return value;
}

function parseArguments(argv) {
  const command = argv[0];
  const options = { repeat: false };
  for (let index = 1; index < argv.length; index += 1) {
    const argument = argv[index];
    if (argument === "--source-repo") {
      options.sourceRepo = optionValue(argv, index, argument);
      index += 1;
    } else if (argument === "--cache") {
      options.cacheDir = optionValue(argv, index, argument);
      index += 1;
    } else if (argument === "--out") {
      options.outDir = optionValue(argv, index, argument);
      index += 1;
    } else if (argument === "--patches") {
      options.patchesDir = optionValue(argv, index, argument);
      index += 1;
    } else if (argument === "--pin") {
      options.pinPath = optionValue(argv, index, argument);
      index += 1;
    } else if (argument === "--timeout-ms") {
      const value = Number(optionValue(argv, index, argument));
      if (!Number.isSafeInteger(value) || value <= 0) throw new SourceError("usage", "--timeout-ms needs a positive integer");
      options.timeoutMs = value;
      index += 1;
    } else if (argument === "--repeat") {
      options.repeat = true;
    } else {
      throw new SourceError("usage", `unknown option ${argument}`);
    }
  }
  if (command !== "prepare" && command !== "verify") {
    throw new SourceError("usage", "usage: source.mjs <prepare|verify --repeat> [--source-repo <checkout>] [--pin <file>]");
  }
  if (command === "verify" && !options.repeat) {
    throw new SourceError("usage", "verify needs --repeat");
  }
  return { command, options };
}

function installCancellation() {
  const stop = (code) => {
    for (const child of CHILDREN) {
      try {
        killGroup(child);
      } catch (error) {
        if (error.code !== "ESRCH") throw error;
      }
    }
    process.exit(code);
  };
  process.once("SIGINT", () => stop(130));
  process.once("SIGTERM", () => stop(143));
}

async function main() {
  installCancellation();
  const { command, options } = parseArguments(process.argv.slice(2));
  if (command === "prepare") {
    const result = await prepare(options);
    console.log(`prepared ${result.outDir} at ${result.commit} (${result.entries} entries, ${result.patches.length} patches)`);
    return;
  }
  const result = await verifyRepeat(options);
  console.log(`verified two preparations of ${result.commit} match (${result.entries} entries, ${result.patches.length} patches)`);
}

if (process.argv[1] !== undefined && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().catch((error) => {
    console.error(`error: ${error.message}`);
    process.exit(1);
  });
}
