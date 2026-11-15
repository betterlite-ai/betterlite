import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import test from "node:test";

import { finalize } from "../../bundle/finalize.mjs";
import { listTree, treeDigest } from "../../bundle/inputs.mjs";
import { EXPORT_FILES, put, scratch, writeApp, writeExport } from "./fixtures.mjs";

const INPUTS = { key: "f".repeat(64), buildId: "b".repeat(20), apiBase: "", builder: "node@sha256:x", patches: ["10-build.patch"] };
const VERSIONS = { node: "v24.19.0", npm: "11.6.0" };

async function setup(t, { files = EXPORT_FILES, license = true } = {}) {
  const root = await scratch(t);
  await writeApp(path.join(root, "app"), { license });
  await writeExport(path.join(root, "out"), files);
  await put(root, "LICENSE", "upstream licence text\n");
  return {
    root,
    options: {
      exported: path.join(root, "out"),
      appRoot: path.join(root, "app"),
      license: path.join(root, "LICENSE"),
      bundle: path.join(root, "bundle"),
      inputs: INPUTS,
      versions: VERSIONS,
    },
  };
}

function without(name) {
  const files = { ...EXPORT_FILES };
  delete files[name];
  return files;
}

test("the bundle holds the export, the notices, the marker and a manifest", async (t) => {
  const { options } = await setup(t);
  const manifest = await finalize(options);
  const ui = path.join(options.bundle, "ui");
  assert.equal(await fs.readFile(path.join(ui, "index.html"), "utf8"), EXPORT_FILES["index.html"]);
  assert.equal(await fs.readFile(path.join(ui, ".litellm_ui_ready"), "utf8"), "");
  const files = await listTree(ui);
  assert.equal(manifest.files, files.length);
  assert.equal(manifest.tree, treeDigest(files));
  assert.equal(manifest.node, "v24.19.0");
  assert.equal(manifest.npm, "11.6.0");
  const written = JSON.parse(await fs.readFile(path.join(options.bundle, "manifest.json"), "utf8"));
  assert.deepEqual(written, manifest);
  assert.equal(Object.keys(written)[0], "key");
  assert.equal(written.key, INPUTS.key);
  assert.equal(written.buildId, INPUTS.buildId);
  assert.deepEqual(written.patches, INPUTS.patches);
});

test("the upstream licence is copied verbatim", async (t) => {
  const { options } = await setup(t);
  await finalize(options);
  assert.equal(await fs.readFile(path.join(options.bundle, "ui/notices/LiteLLM-LICENSE.txt"), "utf8"), "upstream licence text\n");
});

test("the notices carry runtime dependency licences and bundler legal comments only", async (t) => {
  const { options } = await setup(t);
  await finalize(options);
  const text = await fs.readFile(path.join(options.bundle, "ui/notices/npm-dependencies.txt"), "utf8");
  assert.match(text, /==== runtime-lib@1\.2\.3 \(MIT\) ====\n--- LICENSE ---\nruntime-lib licence text\n/);
  assert.match(text, /==== nested-lib@0\.1\.0 \(license not declared\) ====\n--- COPYING\.md ---\nnested terms\n/);
  assert.equal(text.includes("README"), false);
  assert.equal(text.includes("dev only licence"), false);
  assert.equal(text.includes("no-license-file"), false);
  assert.equal(text.includes("linked"), false);
  assert.match(text, /==== bundler legal comments \(3\) ====/);
  assert.equal(text.split("/*! chunk-lib v2 | MIT */").length, 2, "a repeated comment is listed once");
  assert.match(text, /\/\*! css-lib \| MIT \*\//);
  assert.match(text, /\/\*\* @license Apache-2\.0 \*\//);
  assert.equal(text.includes("plain comment"), false);
  assert.equal(text.endsWith("\n"), true);
});

test("a build without any npm licence file is refused", async (t) => {
  const { options } = await setup(t, { license: false });
  await assert.rejects(finalize(options), (error) => error.code === "notices");
});

test("legal comments are read from static scripts and styles only", async (t) => {
  const { options } = await setup(t, {
    files: { ...EXPORT_FILES, "page.html": "<!-- x --> /*! html comment */", "_next/static/media/font.woff2": "/*! binary comment */" },
  });
  await finalize(options);
  const text = await fs.readFile(path.join(options.bundle, "ui/notices/npm-dependencies.txt"), "utf8");
  assert.equal(text.includes("html comment"), false);
  assert.equal(text.includes("binary comment"), false);
});

test("modes are normalised and the marker comes last", async (t) => {
  const { options } = await setup(t);
  await fs.chmod(path.join(options.exported, "index.html"), 0o600);
  await fs.chmod(path.join(options.exported, "login"), 0o700);
  await finalize(options);
  const ui = path.join(options.bundle, "ui");
  assert.equal((await fs.stat(path.join(ui, "index.html"))).mode & 0o777, 0o644);
  assert.equal((await fs.stat(path.join(ui, "login"))).mode & 0o777, 0o755);
  assert.equal((await fs.stat(ui)).mode & 0o777, 0o755);
  assert.equal((await fs.stat(path.join(ui, ".litellm_ui_ready"))).mode & 0o777, 0o644);
  assert.equal((await fs.stat(path.join(ui, "notices"))).mode & 0o777, 0o755);
});

test("two finalisations of one export give the same tree", async (t) => {
  const { options } = await setup(t);
  const first = await finalize(options);
  const firstFiles = await listTree(path.join(options.bundle, "ui"));
  const second = await finalize({ ...options, bundle: path.join(path.dirname(options.bundle), "bundle-2") });
  assert.deepEqual(second, first);
  assert.deepEqual(await listTree(path.join(path.dirname(options.bundle), "bundle-2/ui")), firstFiles);
});

test("an old bundle in the target is replaced", async (t) => {
  const { options } = await setup(t);
  await put(options.bundle, "ui/stale.html", "old");
  await finalize(options);
  await assert.rejects(fs.stat(path.join(options.bundle, "ui/stale.html")));
});

test("a missing page, payload or asset fails the layout check", async (t) => {
  const cases = {
    "index.html": /index\.html/,
    "login/index.html": /login\/index\.html/,
    "teams/index.html": /teams\/index\.html/,
    "assets/icon.svg": /assets\/icon\.svg/,
    "next.svg": /next\.svg/,
    "_next/static/chunks/app.js": /_next\/static\//,
  };
  for (const [name, pattern] of Object.entries(cases)) {
    const files = without(name);
    if (name === "_next/static/chunks/app.js") delete files["_next/static/css/app.css"];
    const { options } = await setup(t, { files });
    await assert.rejects(finalize(options), (error) => error.code === "layout" && pattern.test(error.message), name);
  }
});

test("routes in groups, dynamic segments, slots and private folders need no page", async (t) => {
  const { options } = await setup(t);
  await finalize(options);
  const ui = path.join(options.bundle, "ui");
  for (const absent of ["[teamId]", "@modal", "_private", "(.)photo", "(dashboard)"]) {
    await assert.rejects(fs.stat(path.join(ui, absent)), absent);
  }
});

test("RSC payloads and segment files are required", async (t) => {
  const noText = { ...EXPORT_FILES };
  delete noText["index.txt"];
  delete noText["__next.__PAGE__.txt"];
  const { options: none } = await setup(t, { files: noText });
  await assert.rejects(finalize(none), (error) => error.code === "layout" && /RSC \*\.txt payloads/.test(error.message));
  const noSegments = without("__next.__PAGE__.txt");
  const { options: partial } = await setup(t, { files: noSegments });
  await assert.rejects(finalize(partial), (error) => error.code === "layout" && /__next segment files/.test(error.message));
});

test("a segment file in a __next directory counts", async (t) => {
  const files = without("__next.__PAGE__.txt");
  files["__next.teams/__PAGE__.txt"] = "segment";
  const { options } = await setup(t, { files });
  await finalize(options);
});

test("any logo file in the export is refused", async (t) => {
  for (const logo of ["logo.png", "logo_dark.png", "logo_monogram.png", "logo_monogram_dark.png", "cached_logo.jpg", "assets/logo.png"]) {
    const { options } = await setup(t, { files: { ...EXPORT_FILES, [logo]: "png" } });
    await assert.rejects(finalize(options), (error) => error.code === "logo" && error.message.includes(logo), logo);
  }
});

test("a similar file name is not a logo", async (t) => {
  const { options } = await setup(t, { files: { ...EXPORT_FILES, "assets/logo.svg": "<svg/>", "assets/logo.png.map": "{}" } });
  await finalize(options);
});

test("a symlink in the export is refused", async (t) => {
  const { options } = await setup(t);
  await fs.symlink("/etc/passwd", path.join(options.exported, "leak.txt"));
  await assert.rejects(finalize(options), (error) => error.code === "tree-entry" && /leak\.txt/.test(error.message));
});

test("manifest inputs without a cache key are refused", async (t) => {
  const { options } = await setup(t);
  for (const key of [undefined, "", "abc", "F".repeat(64)]) {
    await assert.rejects(finalize({ ...options, inputs: { ...INPUTS, key } }), (error) => error.code === "inputs");
  }
});

test("the manifest tree digest changes with a file's content, mode and path", () => {
  const file = { path: "a.html", mode: 0o644, sha256: "1".repeat(64) };
  const base = treeDigest([file]);
  assert.notEqual(treeDigest([{ ...file, sha256: "2".repeat(64) }]), base);
  assert.notEqual(treeDigest([{ ...file, mode: 0o755 }]), base);
  assert.notEqual(treeDigest([{ ...file, path: "b.html" }]), base);
  assert.notEqual(treeDigest([file, { ...file, path: "c.html" }]), base);
});

test("listTree lists files sorted by path with mode and digest", async (t) => {
  const root = await scratch(t);
  await put(root, "b/two.txt", "2");
  await put(root, "a.txt", "1");
  await fs.chmod(path.join(root, "a.txt"), 0o640);
  await fs.chmod(path.join(root, "b/two.txt"), 0o644);
  const files = await listTree(root);
  assert.deepEqual(
    files.map((file) => [file.path, file.mode]),
    [
      ["a.txt", 0o640],
      ["b/two.txt", 0o644],
    ],
  );
  assert.equal(files[0].sha256, "6b86b273ff34fce19d6b804eff5a3f5747ada4eaa22f1d49c01e52ddb7875b4b");
});
