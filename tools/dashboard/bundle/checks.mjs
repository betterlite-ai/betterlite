import { execFileSync } from "node:child_process";
import fs from "node:fs/promises";
import path from "node:path";

import { PatchError, readPatchFiles } from "../patches.mjs";
import { APP, listTree } from "./inputs.mjs";

async function patchedSources(patchesDir) {
  const sources = new Set();
  for (const file of await readPatchFiles(patchesDir)) {
    for (const target of file.targets) {
      const relative = target.path.slice(`${APP}/`.length);
      if (target.postimage !== null && /\.tsx?$/.test(relative)) sources.add(relative);
    }
  }
  return [...sources].sort();
}

async function extraTests(testsDir, appRoot) {
  let text;
  try {
    text = await fs.readFile(path.join(testsDir, "upstream-tests.json"), "utf8");
  } catch (error) {
    if (error.code === "ENOENT") return [];
    throw error;
  }
  const list = JSON.parse(text);
  if (!Array.isArray(list) || list.some((entry) => typeof entry !== "string")) {
    throw new PatchError("upstream-tests", "upstream-tests.json must be an array of paths");
  }
  const appReal = await fs.realpath(appRoot);
  for (const entry of list) {
    const inside = entry !== "" && !path.posix.isAbsolute(entry) && path.posix.normalize(entry) === entry && !entry.startsWith("../");
    if (!inside || entry.split("/").includes("node_modules")) {
      throw new PatchError("upstream-tests", `${entry} is outside the prepared application`);
    }
    if (!/\.test\.tsx?$/.test(entry)) throw new PatchError("upstream-tests", `${entry} is not a Vitest file`);
    const real = await fs.realpath(path.join(appRoot, entry)).catch(() => null);
    if (real === null || !real.startsWith(`${appReal}${path.sep}`) || !(await fs.stat(real)).isFile()) {
      throw new PatchError("upstream-tests", `${entry} has no tests in the prepared application`);
    }
  }
  return list;
}

export async function writePlan({ testsDir, appRoot, repoRoot, out }) {
  const stat = await fs.stat(testsDir).catch(() => null);
  if (stat === null || !stat.isDirectory()) throw new PatchError("tests-dir", `${testsDir} is not a directory`);
  await listTree(testsDir);
  const extras = await extraTests(testsDir, appRoot);
  const patched = await patchedSources(path.join(repoRoot, "ui/dashboard/patches"));
  await fs.rm(out, { recursive: true, force: true });
  await fs.mkdir(out, { recursive: true });
  await fs.cp(testsDir, path.join(out, "tests"), { recursive: true });
  const plan = { tests: ["tests/betterlite", ...extras], files: [...patched, "tests/betterlite/**/*.{ts,tsx}"] };
  await fs.writeFile(path.join(out, "plan.json"), `${JSON.stringify(plan, null, 2)}\n`);
  return plan;
}

export function runChecks({ appRoot, plan }) {
  const bin = (name) => path.join(appRoot, "node_modules/.bin", name);
  const run = (name, args) => execFileSync(bin(name), args, { cwd: appRoot, stdio: "inherit" });
  for (const target of plan.tests) run("vitest", ["run", target]);
  run("eslint", plan.files);
  run("prettier", ["--check", ...plan.files]);
}
