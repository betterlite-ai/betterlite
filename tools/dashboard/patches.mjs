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

function digest(bytes) {
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
  const targets = record.targets.map((target) => {
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
  return targets;
}

function splitQuoted(text, start) {
  let value = "";
  let index = start + 1;
  while (index < text.length && text[index] !== '"') {
    const character = text[index];
    if (character !== "\\") {
      value += character;
      index += 1;
      continue;
    }
    const escaped = text[index + 1];
    const escapes = { '"': '"', "\\": "\\", n: "\n", t: "\t", r: "\r", b: "\b", f: "\f", v: "\v" };
    if (Object.hasOwn(escapes, escaped)) {
      value += escapes[escaped];
      index += 2;
      continue;
    }
    const octal = /^[0-7]{1,3}/.exec(text.slice(index + 1));
    if (octal === null) {
      throw new PatchError("patch-form", `unsupported escape in a quoted patch path at offset ${index}`);
    }
    value += String.fromCharCode(Number.parseInt(octal[0], 8));
    index += 1 + octal[0].length;
  }
  if (text[index] !== '"') {
    throw new PatchError("patch-form", "a quoted patch path does not end");
  }
  return { value, end: index + 1 };
}

function twoPaths(text) {
  const first = text.startsWith('"') ? splitQuoted(text, 0) : { value: text.split(" ")[0], end: text.split(" ")[0].length };
  let rest = text.slice(first.end);
  if (!rest.startsWith(" ")) {
    throw new PatchError("patch-form", `cannot separate the two paths in 'diff --git ${text}'`);
  }
  rest = rest.slice(1);
  const second = rest.startsWith('"') ? splitQuoted(rest, 0) : { value: rest.split(" ")[0], end: rest.split(" ")[0].length };
  if (second.end !== rest.length) {
    throw new PatchError("patch-form", `unexpected text after the paths in 'diff --git ${text}'`);
  }
  return [first.value, second.value];
}

function prefixed(value, prefix, patch) {
  if (!value.startsWith(prefix)) {
    throw new PatchError("patch-form", `${patch}: '${value}' does not use the ${prefix} prefix`);
  }
  return relativePath(value.slice(prefix.length), patch);
}

export function patchPaths(text, patch) {
  const paths = new Set();
  for (const line of text.split("\n")) {
    if (line.startsWith("rename from ") || line.startsWith("rename to ")) {
      throw new PatchError("patch-form", `${patch}: renames are not part of the patch contract`);
    }
    if (line.startsWith("copy from ") || line.startsWith("copy to ")) {
      throw new PatchError("patch-form", `${patch}: copies are not part of the patch contract`);
    }
    if (!line.startsWith("diff --git ")) continue;
    const [first, second] = twoPaths(line.slice("diff --git ".length));
    paths.add(prefixed(first, "a/", patch));
    paths.add(prefixed(second, "b/", patch));
  }
  if (paths.size === 0) {
    throw new PatchError("patch-form", `${patch}: no 'diff --git' header names a target`);
  }
  return paths;
}

async function digestOrNull(file) {
  try {
    return digest(await fs.readFile(file));
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

async function run(root, runGit, args, timeoutMs, code, patch) {
  try {
    return await runGit(args, { cwd: root, timeoutMs, ceiling: path.dirname(root) });
  } catch (error) {
    throw new PatchError(code, `${patch}: git apply ${args.includes("--check") ? "--check " : ""}failed: ${error.message}`);
  }
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
  const applied = [];
  for (const name of names) {
    const patchPath = path.join(patchesDir, name);
    const recordPath = patchPath.slice(0, -".patch".length) + ".json";
    let recordText;
    try {
      recordText = await fs.readFile(recordPath, "utf8");
    } catch (error) {
      if (error.code === "ENOENT") {
        throw new PatchError("patch-record", `${name} has no adjacent ${path.basename(recordPath)} record`);
      }
      throw error;
    }
    const targets = readRecord(recordText, name);
    const text = await fs.readFile(patchPath, "utf8");
    const declared = patchPaths(text, name);
    const recorded = new Set(targets.map((target) => target.path));
    for (const file of declared) {
      if (!recorded.has(file)) {
        throw new PatchError("patch-record", `${name}: the patch touches ${file}, which its record does not list`);
      }
    }
    for (const target of targets) {
      if (!declared.has(target.path)) {
        throw new PatchError("patch-record", `${name}: its record lists ${target.path}, which the patch does not touch`);
      }
      if (isProtectedPath(target.path)) {
        throw new PatchError("patch-protected", `${name}: ${target.path} is pinned upstream content and cannot be patched`);
      }
      if (!isAllowedPath(target.path)) {
        throw new PatchError("patch-path", `${name}: ${target.path} is outside the selected dashboard export`);
      }
      await checkTarget(root, target, target.preimage, name, "before the patch");
    }
    await run(root, runGit, ["apply", "--check", "--binary", patchPath], timeoutMs, "patch-check", name);
    await run(root, runGit, ["apply", "--binary", patchPath], timeoutMs, "patch-apply", name);
    for (const target of targets) {
      await checkTarget(root, target, target.postimage, name, "after the patch");
    }
    applied.push(name);
  }
  return applied;
}

export { digest as sha256Hex };
