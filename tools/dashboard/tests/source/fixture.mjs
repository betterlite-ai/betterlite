import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { pathToFileURL } from "node:url";

const GIT_ENVIRONMENT = {
  GIT_CONFIG_GLOBAL: os.devNull,
  GIT_CONFIG_SYSTEM: os.devNull,
  GIT_CONFIG_NOSYSTEM: "1",
  GIT_AUTHOR_NAME: "Fixture",
  GIT_AUTHOR_EMAIL: "fixture@example.invalid",
  GIT_COMMITTER_NAME: "Fixture",
  GIT_COMMITTER_EMAIL: "fixture@example.invalid",
};

export const ARCHIVE_PATHS = [
  "LICENSE",
  "ui",
  "litellm/proxy/cached_logo.jpg",
  "litellm/proxy/logo.png",
  "litellm/proxy/logo_dark.png",
  "litellm/proxy/logo_monogram.png",
  "litellm/proxy/logo_monogram_dark.png",
];

export const FILES = {
  LICENSE: "MIT License\n\nCopyright (c) 2023 Berri AI\n\nPermission is hereby granted, free of charge.\n",
  "ui/Dockerfile": "FROM scratch\n",
  "ui/litellm-dashboard/.npmrc": "ignore-scripts=true\nengine-strict=true\n",
  "ui/litellm-dashboard/README.md": "This is a Next.js project bootstrapped with create-next-app.\n",
  "ui/litellm-dashboard/next.config.mjs": "export default { output: \"standalone\" };\n",
  "ui/litellm-dashboard/package-lock.json": "{\n  \"lockfileVersion\": 3\n}\n",
  "ui/litellm-dashboard/package.json": "{\n  \"name\": \"litellm-dashboard\",\n  \"private\": true\n}\n",
  "ui/litellm-dashboard/src/app/page.tsx": "export default function Page() {\n  return null;\n}\n",
  "ui/litellm-dashboard/_experimental/out/index.html": "<html></html>\n",
  "ui/enterprise/secret.js": "throw new Error(\"proprietary\");\n",
  "litellm/proxy/cached_logo.jpg": "cached-logo",
  "litellm/proxy/logo.png": "logo",
  "litellm/proxy/logo_dark.png": "logo-dark",
  "litellm/proxy/logo_monogram.png": "monogram",
  "litellm/proxy/logo_monogram_dark.png": "monogram-dark",
};

export function hashBytes(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

export function hashText(text) {
  return hashBytes(Buffer.from(text));
}

export function git(cwd, args) {
  return execFileSync("git", args, {
    cwd,
    env: { ...process.env, ...GIT_ENVIRONMENT },
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
  }).trim();
}

function archiveOf(repo, commit, archivePaths) {
  return execFileSync(
    "git",
    ["-C", repo, "-c", "tar.umask=0002", "-c", `core.attributesFile=${os.devNull}`, "archive", "--format=tar", commit, ...archivePaths],
    { env: { ...process.env, ...GIT_ENVIRONMENT }, stdio: ["ignore", "pipe", "pipe"] },
  );
}

async function writeFiles(root, files) {
  for (const [relative, content] of Object.entries(files)) {
    const file = path.join(root, relative);
    await fs.mkdir(path.dirname(file), { recursive: true });
    await fs.writeFile(file, content);
  }
}

export async function createFixture(t) {
  const base = await fs.mkdtemp(path.join(os.tmpdir(), "b9090-source-"));
  t.after(() => fs.rm(base, { recursive: true, force: true }));
  const repo = path.join(base, "upstream");
  await writeFiles(repo, FILES);
  git(repo, ["init", "-q", "-b", "main"]);
  git(repo, ["add", "-A"]);
  git(repo, ["commit", "-q", "-m", "pin"]);
  const commit = git(repo, ["rev-parse", "HEAD"]);
  const pin = {
    repository: "https://example.invalid/litellm.git",
    commit,
    dashboardPath: "ui/litellm-dashboard",
    dashboardTree: git(repo, ["rev-parse", `${commit}:ui/litellm-dashboard`]),
    archivePaths: [...ARCHIVE_PATHS],
    archiveSha256: hashBytes(archiveOf(repo, commit, ARCHIVE_PATHS)),
    excludedPrefixes: ["ui/litellm-dashboard/_experimental/out"],
    protectedFiles: ["LICENSE", "ui/litellm-dashboard/package.json", "ui/litellm-dashboard/package-lock.json", "ui/litellm-dashboard/.npmrc"].map((relative) => ({
      path: relative,
      sha256: hashText(FILES[relative]),
    })),
  };
  const remote = path.join(base, "remote.git");
  git(base, ["clone", "--bare", "--quiet", repo, remote]);
  git(remote, ["config", "uploadpack.allowAnySHA1InWant", "true"]);
  return {
    base,
    repo,
    remote,
    pin,
    files: FILES,
    fetchPin: { ...pin, repository: pathToFileURL(remote).href },
    paths: {
      cacheDir: path.join(base, "cache"),
      outDir: path.join(base, "out"),
      patchesDir: path.join(base, "patches"),
      pinPath: path.join(base, "source.pin.json"),
    },
  };
}

export async function writePatchPair(patchesDir, name, patchText, targets) {
  await fs.mkdir(patchesDir, { recursive: true });
  await fs.writeFile(path.join(patchesDir, name), patchText);
  await fs.writeFile(path.join(patchesDir, name.replace(/\.patch$/, ".json")), JSON.stringify({ targets }, null, 2) + "\n");
}

export function singleLinePatch(relative, before, after) {
  return [
    `diff --git a/${relative} b/${relative}`,
    `--- a/${relative}`,
    `+++ b/${relative}`,
    "@@ -1 +1 @@",
    `-${before.replace(/\n$/, "")}`,
    `+${after.replace(/\n$/, "")}`,
    "",
  ].join("\n");
}

function tarHeader(name, mode, size, type, linkname) {
  const header = Buffer.alloc(512);
  header.write(name, 0, 100, "utf8");
  header.write(`${mode.toString(8).padStart(7, "0")}\0`, 100, 8, "ascii");
  header.write("0000000\0", 108, 8, "ascii");
  header.write("0000000\0", 116, 8, "ascii");
  header.write(`${size.toString(8).padStart(11, "0")}\0`, 124, 12, "ascii");
  header.write("00000000000\0", 136, 12, "ascii");
  header.write("        ", 148, 8, "ascii");
  header.write(type, 156, 1, "ascii");
  header.write(linkname, 157, 100, "utf8");
  header.write("ustar\0", 257, 6, "ascii");
  header.write("00", 263, 2, "ascii");
  let sum = 0;
  for (const byte of header) sum += byte;
  header.write(`${sum.toString(8).padStart(6, "0")}\0 `, 148, 8, "ascii");
  return header;
}

export function paxRecord(key, value) {
  const body = ` ${key}=${value}\n`;
  const bytes = Buffer.byteLength(body);
  let length = bytes + 1;
  for (;;) {
    const total = String(length).length + bytes;
    if (total === length) break;
    length = total;
  }
  return `${length}${body}`;
}

export function buildTar(entries) {
  const parts = [];
  for (const entry of entries) {
    const content = Buffer.from(entry.content ?? "");
    const type = entry.type ?? "0";
    const hasContent = type === "0" || type === "x" || type === "g";
    parts.push(tarHeader(entry.name, entry.mode ?? 0o644, hasContent ? content.length : 0, type, entry.linkname ?? ""));
    if (hasContent) {
      parts.push(content, Buffer.alloc((512 - (content.length % 512)) % 512));
    }
  }
  parts.push(Buffer.alloc(1024));
  return Buffer.concat(parts);
}

export async function exists(target) {
  try {
    await fs.lstat(target);
    return true;
  } catch (error) {
    if (error.code === "ENOENT") return false;
    throw error;
  }
}
