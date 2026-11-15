import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";

import { sha256Hex } from "../patches.mjs";
import { SourceError } from "./errors.mjs";
import { runGit } from "./git.mjs";

const MAX_ARCHIVE_OUTPUT = 512 * 1024 * 1024;

async function verifyCommit(repoDir, pin, timeoutMs) {
  const kind = await runGit(["-C", repoDir, "cat-file", "-t", pin.commit], { timeoutMs, allowFailure: true });
  const objectType = kind.stdout.toString("utf8").trim();
  if (kind.status !== 0 || objectType !== "commit") {
    throw new SourceError("source-commit", `${repoDir} does not contain the pinned commit ${pin.commit}`);
  }
  const peeled = await runGit(["-C", repoDir, "rev-parse", "--verify", "--quiet", `${pin.commit}^{commit}`], { timeoutMs, allowFailure: true });
  if (peeled.status !== 0 || peeled.stdout.toString("utf8").trim() !== pin.commit) {
    throw new SourceError("source-commit", `${pin.commit} is not the pinned commit object; tags and other object kinds are refused`);
  }
  const tree = await runGit(["-C", repoDir, "rev-parse", "--verify", "--quiet", `${pin.commit}:${pin.dashboardPath}`], { timeoutMs, allowFailure: true });
  if (tree.status !== 0 || tree.stdout.toString("utf8").trim() !== pin.dashboardTree) {
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
    { timeoutMs: settings.fetchTimeoutMs, code: "fetch" },
  );
  await verifyCommit(repoDir, pin, settings.commandTimeoutMs);
  return repoDir;
}

export async function resolveSource(settings, pin) {
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

export async function obtainArchive(settings, pin, source) {
  const cacheArchive = path.join(settings.cacheDir, "archive.tar");
  if (source.cached) {
    let cached = null;
    try {
      cached = await fs.readFile(cacheArchive);
    } catch (error) {
      if (error.code !== "ENOENT") throw error;
    }
    if (cached !== null) {
      const actual = sha256Hex(cached);
      if (actual !== pin.archiveSha256) {
        throw new SourceError("cache-corrupt", `${cacheArchive} has SHA-256 ${actual}, the pin expects ${pin.archiveSha256}; delete ${settings.cacheDir} to refetch`);
      }
      return { file: cacheArchive, temporary: false };
    }
  }
  const bytes = await archiveBytes(source.repoDir, pin, settings.archiveTimeoutMs);
  const actual = sha256Hex(bytes);
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
