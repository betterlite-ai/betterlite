import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import test from "node:test";

import { runChecks, writePlan } from "../../bundle/checks.mjs";
import { put, scratch, writeRepo } from "./fixtures.mjs";

async function setup(t, { extras } = {}) {
  const root = await scratch(t);
  await writeRepo(path.join(root, "repo"));
  await put(path.join(root, "app"), "src/lib/known.test.ts", "");
  await put(path.join(root, "app"), "src/lib/view.test.tsx", "");
  await put(path.join(root, "app"), "src/lib/helper.ts", "");
  await put(path.join(root, "app"), "node_modules/pkg/dep.test.ts", "");
  await put(root, "tests/sample.test.ts", "// sample\n");
  if (extras !== undefined) await put(root, "tests/upstream-tests.json", typeof extras === "string" ? extras : JSON.stringify(extras));
  return {
    root,
    options: { testsDir: path.join(root, "tests"), appRoot: path.join(root, "app"), repoRoot: path.join(root, "repo"), out: path.join(root, "out") },
  };
}

test("the plan runs the caller's tests and lints the patched sources and those tests", async (t) => {
  const { root, options } = await setup(t);
  const plan = await writePlan(options);
  assert.deepEqual(plan, {
    tests: ["tests/betterlite"],
    files: ["src/app/layout.tsx", "tests/betterlite/**/*.{ts,tsx}"],
  });
  const written = JSON.parse(await fs.readFile(path.join(root, "out/plan.json"), "utf8"));
  assert.deepEqual(written, plan);
  assert.equal(await fs.readFile(path.join(root, "out/tests/sample.test.ts"), "utf8"), "// sample\n");
});

test("only patched TypeScript sources that still exist are linted", async (t) => {
  const { options } = await setup(t);
  const plan = await writePlan(options);
  assert.equal(plan.files.includes("next.config.mjs"), false, "a config file is not linted as source");
  assert.equal(plan.files.includes("src/gone.tsx"), false, "a deleted file has no postimage");
  assert.equal(plan.files.includes("src/data.json"), false);
});

test("upstream tests are appended after the caller's tests", async (t) => {
  const { options } = await setup(t, { extras: ["src/lib/known.test.ts", "src/lib/view.test.tsx"] });
  const plan = await writePlan(options);
  assert.deepEqual(plan.tests, ["tests/betterlite", "src/lib/known.test.ts", "src/lib/view.test.tsx"]);
});

test("an empty upstream list adds nothing", async (t) => {
  const { options } = await setup(t, { extras: [] });
  assert.deepEqual((await writePlan(options)).tests, ["tests/betterlite"]);
});

test("a stale plan in the output directory is replaced", async (t) => {
  const { root, options } = await setup(t);
  await put(root, "out/old.txt", "old");
  await writePlan(options);
  await assert.rejects(fs.stat(path.join(root, "out/old.txt")));
});

test("upstream-tests.json must be an array of paths", async (t) => {
  for (const extras of [{ tests: [] }, '"src/lib/known.test.ts"',[1], [null], ["src/lib/known.test.ts", 2]]) {
    const { options } = await setup(t, { extras });
    await assert.rejects(writePlan(options), (error) => error.code === "upstream-tests" && /array of paths/.test(error.message), JSON.stringify(extras));
  }
});

test("upstream tests stay inside the prepared application", async (t) => {
  const bad = ["../x.test.ts", "/etc/x.test.ts", "src/../lib/known.test.ts", "", "./src/lib/known.test.ts", "node_modules/pkg/dep.test.ts", "src//lib/known.test.ts"];
  for (const entry of bad) {
    const { options } = await setup(t, { extras: [entry] });
    await assert.rejects(writePlan(options), (error) => error.code === "upstream-tests" && /outside the prepared application/.test(error.message), entry);
  }
});

test("an upstream entry must be a Vitest file that exists", async (t) => {
  const { options: helper } = await setup(t, { extras: ["src/lib/helper.ts"] });
  await assert.rejects(writePlan(helper), (error) => /not a Vitest file/.test(error.message));
  const { options: missing } = await setup(t, { extras: ["src/lib/missing.test.ts"] });
  await assert.rejects(writePlan(missing), (error) => /has no tests in the prepared application/.test(error.message));
  const { root, options: directory } = await setup(t, { extras: ["src/lib/dir.test.ts"] });
  await fs.mkdir(path.join(root, "app/src/lib/dir.test.ts"));
  await assert.rejects(writePlan(directory), (error) => /has no tests in the prepared application/.test(error.message));
});

test("an upstream entry that links out of the application is refused", async (t) => {
  const { root, options } = await setup(t, { extras: ["src/lib/link.test.ts"] });
  await put(root, "elsewhere.test.ts", "");
  await fs.symlink(path.join(root, "elsewhere.test.ts"), path.join(root, "app/src/lib/link.test.ts"));
  await assert.rejects(writePlan(options), (error) => /has no tests in the prepared application/.test(error.message));
});

test("a tests directory that is missing or holds a symlink is refused", async (t) => {
  const { root, options } = await setup(t);
  await assert.rejects(writePlan({ ...options, testsDir: path.join(root, "nowhere") }), (error) => error.code === "tests-dir");
  await assert.rejects(writePlan({ ...options, testsDir: path.join(root, "tests/sample.test.ts") }), (error) => error.code === "tests-dir");
  await fs.symlink("/etc/passwd", path.join(root, "tests/link.ts"));
  await assert.rejects(writePlan(options), (error) => error.code === "tree-entry");
});

async function stubTools(root, failing = "") {
  const bin = path.join(root, "node_modules/.bin");
  for (const name of ["vitest", "eslint", "prettier"]) {
    const exit = name === failing ? "exit 7" : "exit 0";
    await put(bin, name, `#!/bin/sh\necho "${name} $*" >> "${root}/calls.log"\n${exit}\n`);
    await fs.chmod(path.join(bin, name), 0o755);
  }
}

test("checks run every test target, then eslint, then prettier on the planned files", async (t) => {
  const root = await scratch(t);
  await stubTools(root);
  const plan = { tests: ["tests/betterlite", "src/a.test.ts"], files: ["src/app/layout.tsx", "tests/betterlite/**/*.{ts,tsx}"] };
  runChecks({ appRoot: root, plan });
  const calls = (await fs.readFile(path.join(root, "calls.log"), "utf8")).trim().split("\n");
  assert.deepEqual(calls, [
    "vitest run tests/betterlite",
    "vitest run src/a.test.ts",
    "eslint src/app/layout.tsx tests/betterlite/**/*.{ts,tsx}",
    "prettier --check src/app/layout.tsx tests/betterlite/**/*.{ts,tsx}",
  ]);
});

test("a failing tool stops the checks", async (t) => {
  for (const [failing, ran] of [
    ["vitest", ["vitest run tests/betterlite"]],
    ["eslint", ["vitest run tests/betterlite", "eslint src/a.tsx"]],
    ["prettier", ["vitest run tests/betterlite", "eslint src/a.tsx", "prettier --check src/a.tsx"]],
  ]) {
    const root = await scratch(t);
    await stubTools(root, failing);
    assert.throws(() => runChecks({ appRoot: root, plan: { tests: ["tests/betterlite"], files: ["src/a.tsx"] } }), (error) => error.status === 7);
    assert.deepEqual((await fs.readFile(path.join(root, "calls.log"), "utf8")).trim().split("\n"), ran);
  }
});
