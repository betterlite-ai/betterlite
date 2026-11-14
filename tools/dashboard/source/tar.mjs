import fs from "node:fs/promises";
import path from "node:path";

import { SourceError } from "./errors.mjs";

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

function nameFromHeader(header) {
  const name = readString(header, 0, 100);
  if (readString(header, 257, 5) !== "ustar") return name;
  const prefix = readString(header, 345, 155);
  return prefix === "" ? name : `${prefix}/${name}`;
}

function applyPax(entry, pending) {
  if (pending.has("path")) entry.name = pending.get("path");
  if (pending.has("linkpath")) entry.linkname = pending.get("linkpath");
  if (!pending.has("size")) return;
  const size = Number.parseInt(pending.get("size"), 10);
  if (!Number.isSafeInteger(size) || size < 0) {
    throw new SourceError("tar-invalid", `${entry.name}: its pax size is not a number`);
  }
  entry.size = size;
}

function readTarEntry(bytes, offset, pending) {
  const header = bytes.subarray(offset, offset + 512);
  if (header.every((byte) => byte === 0)) return null;
  verifyChecksum(header);
  const size = readOctal(header, 124, 12, "size");
  const type = String.fromCharCode(header[156] === 0 ? 0x30 : header[156]);
  const bodyStart = offset + 512;
  if (type === "x" || type === "g") {
    if (bodyStart + size > bytes.length) {
      throw new SourceError("tar-invalid", "a pax header runs past the end of the archive");
    }
    const next = bodyStart + Math.ceil(size / 512) * 512;
    return { meta: true, next, pending: type === "x" ? paxRecords(bytes.subarray(bodyStart, bodyStart + size)) : null };
  }
  const entry = {
    name: nameFromHeader(header),
    mode: readOctal(header, 100, 8, "mode"),
    size,
    type,
    linkname: readString(header, 157, 100),
  };
  if (pending !== null) applyPax(entry, pending);
  if (bodyStart + entry.size > bytes.length) {
    throw new SourceError("tar-invalid", `${entry.name} runs past the end of the archive`);
  }
  entry.payload = bytes.subarray(bodyStart, bodyStart + entry.size);
  entry.next = bodyStart + Math.ceil(entry.size / 512) * 512;
  return entry;
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

async function writeTarEntry(root, entry, pin, directoryModes) {
  const directory = entry.type === "5";
  const relative = relativeEntry(entry.name, directory);
  if (relative === null) {
    if (directory) return false;
    throw new SourceError("tar-invalid", "a tar entry has an empty name");
  }
  if (isExcludedPath(relative, pin)) return false;
  if (!isSelectedPath(relative, pin)) {
    throw new SourceError("unselected-path", `tar entry ${relative} is outside the selected export`);
  }
  if (directory) {
    await writeDirectory(root, relative, entry.mode, directoryModes);
    return true;
  }
  if (entry.type === "0") {
    await writeEntry(root, relative, entry.mode, entry.payload);
    return true;
  }
  if (entry.type === "2") {
    const absolute = path.join(root, relative);
    checkLink(root, absolute, entry.linkname);
    await fs.mkdir(path.dirname(absolute), { recursive: true });
    await fs.symlink(entry.linkname, absolute);
    return true;
  }
  throw new SourceError("unsupported-entry", `tar entry ${relative} has unsupported type ${entry.type}`);
}

export async function extractArchive(bytes, destDir, pin) {
  const root = path.resolve(destDir);
  await fs.mkdir(root, { recursive: true });
  const directoryModes = [];
  let offset = 0;
  let pending = null;
  let written = 0;
  while (offset + 512 <= bytes.length) {
    const entry = readTarEntry(bytes, offset, pending);
    if (entry === null) break;
    offset = entry.next;
    if (entry.meta) {
      pending = entry.pending;
      continue;
    }
    pending = null;
    if (await writeTarEntry(root, entry, pin, directoryModes)) written += 1;
  }
  for (const [absolute, mode] of directoryModes) {
    await fs.chmod(absolute, mode);
  }
  return written;
}
