import { createHash } from "node:crypto";
import fs from "node:fs/promises";
import path from "node:path";

const HASH = /^[0-9a-f]{64}$/;

export class PatchError extends Error {
  constructor(code, message) {
    super(message);
    this.name = "PatchError";
    this.code = code;
  }
}

export function sha256Hex(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

function relativePath(value, patch) {
  if (typeof value !== "string" || value.length === 0 || value.includes("\0") || path.posix.isAbsolute(value)) {
    throw new PatchError("patch-path", `${patch}: ${JSON.stringify(value)} is not a relative path`);
  }
  const segments = value.split("/");
  if (segments.some((segment) => segment === "" || segment === "." || segment === "..")) {
    throw new PatchError("patch-path", `${patch}: ${JSON.stringify(value)} escapes or is not normalized`);
  }
  return segments.join("/");
}

function hashValue(value, key, patch) {
  if (value === null) return null;
  if (typeof value !== "string" || !HASH.test(value)) {
    throw new PatchError("patch-record", `${patch}: ${key} must be a SHA-256 hex digest or null`);
  }
  return value;
}

function readRecord(text, patch) {
  let record;
  try {
    record = JSON.parse(text);
  } catch (error) {
    throw new PatchError("patch-record", `${patch}: its .json record is not valid JSON: ${error.message}`);
  }
  if (record === null || typeof record !== "object" || !Array.isArray(record.targets)) {
    throw new PatchError("patch-record", `${patch}: its .json record needs a targets array`);
  }
  return record.targets.map((target) => {
    if (target === null || typeof target !== "object") {
      throw new PatchError("patch-record", `${patch}: every target must be an object`);
    }
    const file = relativePath(target.path, patch);
    return {
      path: file,
      preimage: hashValue(target.preimage, `${file} preimage`, patch),
      postimage: hashValue(target.postimage, `${file} postimage`, patch),
    };
  });
}

function targetPaths(output, patch) {
  const fields = output.toString("utf8").split("\0");
  const summary = fields.pop();
  const touched = new Set();
  for (const field of fields) {
    const parts = field.split("\t");
    if (parts.length !== 3) {
      throw new PatchError("patch-form", `${patch}: git apply printed an unexpected numstat record`);
    }
    touched.add(relativePath(parts[2], patch));
  }
  for (const line of summary.split("\n")) {
    const text = line.trimStart();
    if (text.startsWith("rename ") || text.startsWith("copy ")) {
      throw new PatchError("patch-form", `${patch}: renames and copies are not part of the patch contract`);
    }
  }
  if (touched.size === 0) {
    throw new PatchError("patch-form", `${patch}: git apply reports no target path`);
  }
  return touched;
}

async function digestOrNull(file) {
  try {
    return sha256Hex(await fs.readFile(file));
  } catch (error) {
    if (error.code === "ENOENT") return null;
    throw error;
  }
}

async function checkTarget(root, target, expected, patch, side) {
  const actual = await digestOrNull(path.join(root, target.path));
  if (actual === expected) return;
  const seen = actual === null ? "absent" : actual;
  const want = expected === null ? "absent" : expected;
  throw new PatchError("patch-hash", `${patch}: ${target.path} ${side} is ${seen}, the record says ${want}`);
}

async function readRecordFile(recordPath, patch) {
  try {
    return readRecord(await fs.readFile(recordPath, "utf8"), patch);
  } catch (error) {
    if (error.code === "ENOENT") {
      throw new PatchError("patch-record", `${patch} has no adjacent ${path.basename(recordPath)} record`);
    }
    throw error;
  }
}

async function runApply({ root, runGit, args, timeoutMs, code, patch }) {
  try {
    return await runGit(args, { cwd: root, timeoutMs, ceiling: path.dirname(root) });
  } catch (error) {
    throw new PatchError(code, `${patch}: git apply failed: ${error.message}`);
  }
}

async function checkTargets({ root, patch, targets, declared, isAllowedPath, isProtectedPath }) {
  const recorded = new Set(targets.map((target) => target.path));
  for (const file of declared) {
    if (!recorded.has(file)) {
      throw new PatchError("patch-record", `${patch}: the patch touches ${file}, which its record does not list`);
    }
  }
  for (const target of targets) {
    if (!declared.has(target.path)) {
      throw new PatchError("patch-record", `${patch}: its record lists ${target.path}, which the patch does not touch`);
    }
    if (isProtectedPath(target.path)) {
      throw new PatchError("patch-protected", `${patch}: ${target.path} is pinned upstream content and cannot be patched`);
    }
    if (!isAllowedPath(target.path)) {
      throw new PatchError("patch-path", `${patch}: ${target.path} is outside the selected dashboard export`);
    }
    await checkTarget(root, target, target.preimage, patch, "before the patch");
  }
}

async function applyOnePatch({ root, name, patchesDir, isAllowedPath, isProtectedPath, runGit, timeoutMs }) {
  const patchPath = path.join(patchesDir, name);
  const targets = await readRecordFile(patchPath.slice(0, -".patch".length) + ".json", name);
  const check = await runApply({
    root, runGit, timeoutMs, patch: name, code: "patch-check",
    args: ["apply", "--check", "--binary", "--numstat", "--summary", "-z", patchPath],
  });
  const declared = targetPaths(check.stdout, name);
  await checkTargets({ root, patch: name, targets, declared, isAllowedPath, isProtectedPath });
  await runApply({ root, runGit, args: ["apply", "--binary", patchPath], timeoutMs, code: "patch-apply", patch: name });
  for (const target of targets) {
    await checkTarget(root, target, target.postimage, name, "after the patch");
  }
  return name;
}

export async function applyPatches({ root, patchesDir, isAllowedPath, isProtectedPath, runGit, timeoutMs }) {
  let entries;
  try {
    entries = await fs.readdir(patchesDir, { withFileTypes: true });
  } catch (error) {
    if (error.code === "ENOENT") return [];
    throw error;
  }
  const names = entries
    .filter((entry) => entry.isFile() && entry.name.endsWith(".patch"))
    .map((entry) => entry.name)
    .sort();
  for (const name of names) {
    await applyOnePatch({ root, name, patchesDir, isAllowedPath, isProtectedPath, runGit, timeoutMs });
  }
  return names;
}
