import assert from "node:assert/strict";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { test } from "node:test";

import { extractArchive } from "../../source/tar.mjs";
import { buildTar, createFixture, exists, hashBytes, paxRecord } from "./fixture.mjs";

async function scratch(t) {
  const directory = await fs.mkdtemp(path.join(os.tmpdir(), "b9090-tar-"));
  t.after(() => fs.rm(directory, { recursive: true, force: true }));
  return directory;
}

async function rejected(promise, code) {
  await assert.rejects(promise, (error) => {
    assert.equal(error.code, code, error.message);
    return true;
  });
}

test("extraction rejects a symlink that escapes the destination", async (t) => {
  const fixture = await createFixture(t);
  const directory = await scratch(t);
  const destination = path.join(directory, "tree");
  const tar = buildTar([{ name: "ui/evil", type: "2", linkname: "../../outside.txt" }]);
  await rejected(extractArchive(tar, destination, fixture.pin), "unsafe-link");
  assert.equal(await exists(path.join(directory, "outside.txt")), false);
});

test("extraction rejects an entry path that escapes the destination", async (t) => {
  const fixture = await createFixture(t);
  const directory = await scratch(t);
  const tar = buildTar([{ name: "../escape.txt", content: "escaped\n" }]);
  await rejected(extractArchive(tar, path.join(directory, "tree"), fixture.pin), "unsafe-path");
  assert.equal(await exists(path.join(directory, "escape.txt")), false);
});

test("extraction rejects an entry that is outside the selected export", async (t) => {
  const fixture = await createFixture(t);
  const directory = await scratch(t);
  const tar = buildTar([{ name: "README.md", content: "unselected\n" }]);
  await rejected(extractArchive(tar, path.join(directory, "tree"), fixture.pin), "unselected-path");
  assert.equal(await exists(path.join(directory, "tree", "README.md")), false);
});

test("extraction rejects a header whose checksum does not match", async (t) => {
  const fixture = await createFixture(t);
  const directory = await scratch(t);
  const tar = buildTar([{ name: "LICENSE", content: "MIT\n" }]);
  tar[0] ^= 0xff;
  await rejected(extractArchive(tar, path.join(directory, "tree"), fixture.pin), "tar-invalid");
});

test("extraction writes selected files and skips the excluded subtrees", async (t) => {
  const fixture = await createFixture(t);
  const directory = await scratch(t);
  const tar = buildTar([
    { name: "LICENSE", content: "MIT\n" },
    { name: "ui/", type: "5", mode: 0o775 },
    { name: "ui/litellm-dashboard/", type: "5", mode: 0o775 },
    { name: "ui/litellm-dashboard/page.tsx", content: "export default 1;\n", mode: 0o664 },
    { name: "ui/litellm-dashboard/_experimental/out/index.html", content: "<html></html>\n" },
    { name: "litellm/", type: "5", mode: 0o775 },
    { name: "litellm/proxy/", type: "5", mode: 0o775 },
    { name: "litellm/proxy/logo.png", content: "logo", mode: 0o664 },
  ]);
  const written = await extractArchive(tar, path.join(directory, "tree"), fixture.pin);
  const tree = path.join(directory, "tree");
  assert.equal(await fs.readFile(path.join(tree, "LICENSE"), "utf8"), "MIT\n");
  assert.equal(await fs.readFile(path.join(tree, "ui/litellm-dashboard/page.tsx"), "utf8"), "export default 1;\n");
  assert.equal((await fs.stat(path.join(tree, "ui/litellm-dashboard/page.tsx"))).mode & 0o777, 0o664);
  assert.equal((await fs.stat(path.join(tree, "ui"))).mode & 0o777, 0o775);
  assert.equal(await exists(path.join(tree, "ui/litellm-dashboard/_experimental/out")), false);
  assert.equal(written, 7);
});

test("extraction applies a pax path to the next entry", async (t) => {
  const fixture = await createFixture(t);
  const directory = await scratch(t);
  const long = `ui/${"a".repeat(120)}.txt`;
  const tar = buildTar([
    { name: "pax-header", type: "x", content: paxRecord("path", long) },
    { name: "short.txt", content: "long name\n", mode: 0o664 },
  ]);
  await extractArchive(tar, path.join(directory, "tree"), fixture.pin);
  assert.equal(await fs.readFile(path.join(directory, "tree", long), "utf8"), "long name\n");
  assert.equal(await exists(path.join(directory, "tree", "short.txt")), false);
});

test("extraction accepts a safe relative symlink", async (t) => {
  const fixture = await createFixture(t);
  const directory = await scratch(t);
  const tar = buildTar([
    { name: "ui/target.txt", content: "target\n" },
    { name: "ui/link.txt", type: "2", linkname: "target.txt" },
  ]);
  await extractArchive(tar, path.join(directory, "tree"), fixture.pin);
  assert.equal(await fs.readFile(path.join(directory, "tree", "ui/link.txt"), "utf8"), "target\n");
  assert.equal(hashBytes(Buffer.from(await fs.readlink(path.join(directory, "tree", "ui/link.txt")))), hashBytes(Buffer.from("target.txt")));
});
