import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import fs from "node:fs/promises";
import path from "node:path";
import test from "node:test";

import { listTree } from "../../bundle/inputs.mjs";
import { installBundle } from "../../install.mjs";
import { REPO_ROOT, put, scratch } from "./fixtures.mjs";

async function setup(t) {
  const root = await scratch(t);
  const bundle = path.join(root, "dist");
  await put(bundle, "manifest.json", "{}\n");
  await put(bundle, "ui/index.html", "new home");
  await put(bundle, "ui/_next/static/new/chunk.js", "new");
  await put(bundle, "ui/.litellm_ui_ready", "");
  await put(root, "prefix/bin/betterlite", "#!/bin/sh\n");
  return { root, bundle, bin: path.join(root, "prefix/bin/betterlite"), target: path.join(root, "prefix/bin/ui") };
}

test("the bundle's ui lands beside the binary", async (t) => {
  const { bundle, bin, target } = await setup(t);
  assert.equal(await installBundle({ bundle, bin }), target);
  assert.deepEqual(await listTree(target), await listTree(path.join(bundle, "ui")));
});

test("a stale sidecar is replaced, not merged", async (t) => {
  const { bundle, bin, target } = await setup(t);
  await put(target, "_next/static/stale/chunk.js", "old");
  await put(target, "old-route/index.html", "old");
  await put(target, "index.html", "old home");
  await installBundle({ bundle, bin });
  const paths = (await listTree(target)).map((file) => file.path);
  assert.deepEqual(paths, [".litellm_ui_ready", "_next/static/new/chunk.js", "index.html"]);
  assert.equal(await fs.readFile(path.join(target, "index.html"), "utf8"), "new home");
});

test("installing twice leaves the same sidecar and no leftovers", async (t) => {
  const { bundle, bin, target } = await setup(t);
  await installBundle({ bundle, bin });
  const first = await listTree(target);
  await installBundle({ bundle, bin });
  assert.deepEqual(await listTree(target), first);
  assert.deepEqual((await fs.readdir(path.dirname(bin))).sort(), ["betterlite", "ui"]);
});

test("the bundle itself is left as it was", async (t) => {
  const { bundle, bin } = await setup(t);
  const before = await listTree(bundle);
  await installBundle({ bundle, bin });
  assert.deepEqual(await listTree(bundle), before);
});

test("a bundle without the ready marker or manifest is refused and nothing is touched", async (t) => {
  for (const missing of ["ui/.litellm_ui_ready", "manifest.json"]) {
    const { bundle, bin, target } = await setup(t);
    await put(target, "index.html", "old home");
    await fs.rm(path.join(bundle, missing));
    await assert.rejects(installBundle({ bundle, bin }), (error) => error.code === "bundle");
    assert.equal(await fs.readFile(path.join(target, "index.html"), "utf8"), "old home");
  }
});

test("a binary that is not a file is refused", async (t) => {
  const { root, bundle } = await setup(t);
  await assert.rejects(installBundle({ bundle, bin: path.join(root, "prefix/bin/missing") }), (error) => error.code === "bin");
  await assert.rejects(installBundle({ bundle, bin: path.join(root, "prefix/bin") }), (error) => error.code === "bin");
});

test("a symlink in the bundle is refused before the old sidecar moves", async (t) => {
  const { bundle, bin, target } = await setup(t);
  await put(target, "index.html", "old home");
  await fs.symlink("/etc/passwd", path.join(bundle, "ui/link.txt"));
  await assert.rejects(installBundle({ bundle, bin }), (error) => error.code === "tree-entry");
  assert.equal(await fs.readFile(path.join(target, "index.html"), "utf8"), "old home");
});

test("the command line installs and reports the target", async (t) => {
  const { bundle, bin, target } = await setup(t);
  const output = execFileSync("node", [path.join(REPO_ROOT, "tools/dashboard/install.mjs"), "--bundle", bundle, "--bin", bin], { encoding: "utf8" });
  assert.equal(output, `installed ${target}\n`);
  assert.equal(await fs.readFile(path.join(target, "index.html"), "utf8"), "new home");
});

test("the command line needs both options", () => {
  const result = spawnSync("node", [path.join(REPO_ROOT, "tools/dashboard/install.mjs"), "--bundle", "x"], { encoding: "utf8" });
  assert.equal(result.status, 2);
  assert.match(result.stderr, /usage: install\.mjs/);
});

test("the command line exits non-zero for a bundle that is not finished", async (t) => {
  const { root, bin } = await setup(t);
  const result = spawnSync("node", [path.join(REPO_ROOT, "tools/dashboard/install.mjs"), "--bundle", path.join(root, "nothing"), "--bin", bin], {
    encoding: "utf8",
  });
  assert.equal(result.status, 1);
  assert.match(result.stderr, /is not a finished dashboard bundle/);
});
