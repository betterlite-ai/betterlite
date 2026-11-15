import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import fs from "node:fs/promises";
import path from "node:path";
import test from "node:test";

import { bundleInputs, cacheFlags, checkApiBase } from "../../bundle/inputs.mjs";
import { PatchError } from "../../patches.mjs";
import { PINNED_IMAGE, REPO_ROOT, put, scratch, writeRepo } from "./fixtures.mjs";

async function inputsFor(t, { overrides = {}, apiBase = "" } = {}) {
  const root = await scratch(t);
  await writeRepo(root, overrides);
  return bundleInputs({ repoRoot: root, apiBase });
}

test("the same inputs give the same key and build id", async (t) => {
  const first = await inputsFor(t);
  const second = await inputsFor(t);
  assert.deepEqual(second, first);
  assert.match(first.key, /^[0-9a-f]{64}$/);
  assert.match(first.buildId, /^[0-9a-f]{20}$/);
  assert.equal(first.builder, PINNED_IMAGE);
  assert.deepEqual(first.patches, ["10-build.patch"]);
  assert.deepEqual(first.source, { commit: "c".repeat(40), archiveSha256: "a".repeat(64) });
});

test("every input the build reads changes the key", async (t) => {
  const base = await inputsFor(t);
  const changes = {
    "a patch": { "ui/dashboard/patches/10-build.patch": "other patch\n" },
    "a patch record": { "ui/dashboard/patches/10-build.json": JSON.stringify({ targets: [] }) },
    "another patch": {
      "ui/dashboard/patches/20-more.patch": "more\n",
      "ui/dashboard/patches/20-more.json": JSON.stringify({ targets: [] }),
    },
    "the Dockerfile": { "packaging/ui/Dockerfile": `ARG UI_BUILD_IMAGE=${PINNED_IMAGE}\nRUN true\n` },
    "the builder image": { "packaging/ui/Dockerfile": `ARG UI_BUILD_IMAGE=node:24@sha256:${"e".repeat(64)}\n` },
    "build.sh": { "tools/dashboard/build.sh": "#!/bin/sh\n" },
    "bundle.mjs": { "tools/dashboard/bundle.mjs": "// changed\n" },
    "patches.mjs": { "tools/dashboard/patches.mjs": "// changed patches\n" },
    "a bundle module": { "tools/dashboard/bundle/checks.mjs": "// changed checks\n" },
    "a new bundle module": { "tools/dashboard/bundle/extra.mjs": "// extra\n" },
    "the pinned archive": {
      "tools/dashboard/source.pin.json": JSON.stringify({
        commit: "c".repeat(40),
        archiveSha256: "b".repeat(64),
        protectedFiles: [
          { path: "ui/litellm-dashboard/package-lock.json", sha256: "1".repeat(64) },
          { path: "ui/litellm-dashboard/.npmrc", sha256: "2".repeat(64) },
        ],
      }),
    },
    "the lock file hash": {
      "tools/dashboard/source.pin.json": JSON.stringify({
        commit: "c".repeat(40),
        archiveSha256: "a".repeat(64),
        protectedFiles: [
          { path: "ui/litellm-dashboard/package-lock.json", sha256: "3".repeat(64) },
          { path: "ui/litellm-dashboard/.npmrc", sha256: "2".repeat(64) },
        ],
      }),
    },
    ".npmrc hash": {
      "tools/dashboard/source.pin.json": JSON.stringify({
        commit: "c".repeat(40),
        archiveSha256: "a".repeat(64),
        protectedFiles: [
          { path: "ui/litellm-dashboard/package-lock.json", sha256: "1".repeat(64) },
          { path: "ui/litellm-dashboard/.npmrc", sha256: "4".repeat(64) },
        ],
      }),
    },
  };
  const seen = new Set([base.key]);
  for (const [name, overrides] of Object.entries(changes)) {
    const changed = await inputsFor(t, { overrides });
    assert.notEqual(changed.key, base.key, `${name} must change the key`);
    assert.equal(seen.has(changed.key), false, `${name} must give a key of its own`);
    seen.add(changed.key);
  }
});

test("the API base changes the key and the build id", async (t) => {
  const base = await inputsFor(t);
  const withBase = await inputsFor(t, { apiBase: "https://api.example.test" });
  assert.notEqual(withBase.key, base.key);
  assert.notEqual(withBase.buildId, base.buildId);
  assert.equal(withBase.apiBase, "https://api.example.test");
  const other = await inputsFor(t, { apiBase: "https://api.example.test/v1" });
  assert.notEqual(other.key, withBase.key);
});

test("a patch changes the build id", async (t) => {
  const base = await inputsFor(t);
  const changed = await inputsFor(t, { overrides: { "ui/dashboard/patches/10-build.patch": "x\n" } });
  assert.notEqual(changed.buildId, base.buildId);
});

test("the build id leaves the builder and scripts out", async (t) => {
  const base = await inputsFor(t);
  const changed = await inputsFor(t, { overrides: { "tools/dashboard/build.sh": "#!/bin/sh\n" } });
  assert.equal(changed.buildId, base.buildId);
});

test("no patches directory is no patches", async (t) => {
  const root = await scratch(t);
  await writeRepo(root);
  await fs.rm(`${root}/ui/dashboard/patches`, { recursive: true });
  const inputs = await bundleInputs({ repoRoot: root });
  assert.deepEqual(inputs.patches, []);
});

test("a directory named like a patch is not a patch", async (t) => {
  const root = await scratch(t);
  await writeRepo(root);
  await fs.mkdir(path.join(root, "ui/dashboard/patches/20-fake.patch"));
  const inputs = await bundleInputs({ repoRoot: root });
  assert.deepEqual(inputs.patches, ["10-build.patch"]);
});

test("a patch without its record is refused", async (t) => {
  const root = await scratch(t);
  await writeRepo(root);
  await put(root, "ui/dashboard/patches/30-lonely.patch", "x\n");
  await assert.rejects(bundleInputs({ repoRoot: root }), (error) => error.code === "patch-record" && /30-lonely\.json/.test(error.message));
});

test("a Dockerfile without a digest-pinned builder is refused", async (t) => {
  for (const line of ["ARG UI_BUILD_IMAGE=node:24-alpine\n", `ARG UI_BUILD_IMAGE=node@sha256:${"d".repeat(63)}\n`, "FROM node\n"]) {
    const root = await scratch(t);
    await writeRepo(root, { "packaging/ui/Dockerfile": line });
    await assert.rejects(bundleInputs({ repoRoot: root }), (error) => error.code === "builder");
  }
});

test("a pin without the lock file hash is refused", async (t) => {
  const root = await scratch(t);
  await writeRepo(root, {
    "tools/dashboard/source.pin.json": JSON.stringify({ commit: "c", archiveSha256: "a", protectedFiles: [] }),
  });
  await assert.rejects(bundleInputs({ repoRoot: root }), (error) => error.code === "pin" && /package-lock\.json/.test(error.message));
});

test("only http and https API bases without credentials pass", () => {
  checkApiBase("");
  checkApiBase("http://localhost:4000");
  checkApiBase("https://api.example.test/base");
  for (const bad of ["api.example.test", "ftp://example.test", "https://user:pw@example.test", "https://user@example.test", "https://exa mple.test", "javascript:alert(1)"]) {
    assert.throws(() => checkApiBase(bad), PatchError, bad);
  }
});

test("the real repository has a key that ignores the environment", () => {
  const run = (env) =>
    execFileSync("node", [`${REPO_ROOT}/tools/dashboard/bundle.mjs`, "key"], { encoding: "utf8", env: { PATH: process.env.PATH, ...env } });
  const key = run({});
  assert.match(key, /^[0-9a-f]{64}\n$/);
  assert.equal(run({ UI_BASE_PATH: "/ignored", NEXT_PUBLIC_BASE_URL: "https://leak.test", NEXT_PUBLIC_USE_REWRITES: "true" }), key);
});

test("the cache is off unless DASHBOARD_BUILDKIT_CACHE asks for it", () => {
  const actions = { ACTIONS_RESULTS_URL: "https://results.test/", ACTIONS_RUNTIME_TOKEN: "runtime-token-value" };
  assert.deepEqual(cacheFlags({}), []);
  assert.deepEqual(cacheFlags({ ...actions }), []);
  assert.deepEqual(cacheFlags({ ...actions, DASHBOARD_BUILDKIT_CACHE: "" }), []);
});

test("read mode only reads the gha cache and read-write also writes it", () => {
  const actions = { ACTIONS_RESULTS_URL: "https://results.test/", ACTIONS_RUNTIME_TOKEN: "runtime-token-value" };
  assert.deepEqual(cacheFlags({ ...actions, DASHBOARD_BUILDKIT_CACHE: "read" }), ["--cache-from", "type=gha,scope=dashboard"]);
  assert.deepEqual(cacheFlags({ ...actions, DASHBOARD_BUILDKIT_CACHE: "read-write" }), [
    "--cache-from",
    "type=gha,scope=dashboard",
    "--cache-to",
    "type=gha,scope=dashboard,mode=max",
  ]);
});

test("the cache needs both Actions variables and never carries the token", () => {
  for (const env of [
    { DASHBOARD_BUILDKIT_CACHE: "read-write", ACTIONS_RESULTS_URL: "https://results.test/" },
    { DASHBOARD_BUILDKIT_CACHE: "read-write", ACTIONS_RUNTIME_TOKEN: "runtime-token-value" },
    { DASHBOARD_BUILDKIT_CACHE: "read", ACTIONS_RESULTS_URL: "", ACTIONS_RUNTIME_TOKEN: "runtime-token-value" },
  ]) {
    assert.deepEqual(cacheFlags(env), []);
  }
  const flags = cacheFlags({
    DASHBOARD_BUILDKIT_CACHE: "read-write",
    ACTIONS_RESULTS_URL: "https://results.test/",
    ACTIONS_RUNTIME_TOKEN: "runtime-token-value",
  });
  assert.equal(flags.join(" ").includes("runtime-token-value"), false);
  assert.equal(flags.join(" ").includes("results.test"), false);
});

test("an unknown cache mode is refused", () => {
  assert.throws(() => cacheFlags({ DASHBOARD_BUILDKIT_CACHE: "write" }), (error) => error.code === "cache-mode" && /"write"/.test(error.message));
});
