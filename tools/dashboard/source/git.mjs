import { spawn } from "node:child_process";
import os from "node:os";

import { SourceError } from "./errors.mjs";

const MAX_SMALL_OUTPUT = 4 * 1024 * 1024;
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

function settleChild(child, timer, callback) {
  clearTimeout(timer);
  CHILDREN.delete(child);
  callback();
}

function captureChunk(child, args, chunk, state, fail) {
  state.captured += chunk.length;
  if (state.captured > state.limit) {
    killGroup(child);
    fail(new SourceError("output-limit", `git ${args[0]} wrote more than ${state.limit} bytes`));
    return;
  }
  state.chunks.push(chunk);
}

function collectGit(child, args, options) {
  const state = { chunks: [], captured: 0, stderr: "", limit: options.maxStdoutBytes ?? MAX_SMALL_OUTPUT, timedOut: false, timer: null };
  return new Promise((resolve, reject) => {
    const fail = (error) => settleChild(child, state.timer, () => reject(error));
    state.timer = setTimeout(() => {
      state.timedOut = true;
      killGroup(child);
    }, options.timeoutMs);
    child.stdout.on("data", (chunk) => captureChunk(child, args, chunk, state, fail));
    child.stderr.setEncoding("utf8");
    child.stderr.on("data", (chunk) => {
      if (state.stderr.length < 8192) state.stderr += chunk;
    });
    child.on("error", (error) => fail(new SourceError("git-spawn", `could not run git: ${error.message}`)));
    child.on("close", (status) => settleChild(child, state.timer, () => {
      if (state.timedOut) {
        reject(new SourceError("timeout", `git ${args[0]} did not finish within ${options.timeoutMs} ms`));
        return;
      }
      resolve({ status, stdout: Buffer.concat(state.chunks), stderr: state.stderr.trim() });
    }));
  });
}

function spawnGit(args, options) {
  const child = spawn("git", args, {
    cwd: options.cwd,
    env: isolatedEnvironment(options.env),
    detached: true,
    stdio: ["ignore", "pipe", "pipe"],
  });
  CHILDREN.add(child);
  return collectGit(child, args, options);
}

export async function runGit(args, options = {}) {
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

function stopAll(code) {
  for (const child of CHILDREN) {
    try {
      killGroup(child);
    } catch (error) {
      if (error.code !== "ESRCH") throw error;
    }
  }
  process.exit(code);
}

export function installCancellation() {
  process.once("SIGINT", () => stopAll(130));
  process.once("SIGTERM", () => stopAll(143));
}
