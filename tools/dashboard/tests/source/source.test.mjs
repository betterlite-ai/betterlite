import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { test } from "node:test";

import { fileURLToPath } from "node:url";

import { compareTrees, prepare, verifyRepeat } from "../../source.mjs";
import { createFixture, exists, git, hashBytes, hashText, singleLinePatch, writePatchPair } from "./fixture.mjs";

const SOURCE = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../../source.mjs");

async function rejected(promise, code) {
  await assert.rejects(promise, (error) => {
    assert.equal(error.code, code, error.message);
    return true;
  });
}

test("prepare publishes the selected export and leaves proprietary subtrees out", async (t) => {
  const fixture = await createFixture(t);
  const result = await prepare({
    pin: fixture.pin,
    sourceRepo: fixture.repo,
    cacheDir: fixture.paths.cacheDir,
    outDir: fixture.paths.outDir,
    patchesDir: fixture.paths.patchesDir,
  });
  assert.equal(result.commit, fixture.pin.commit);
  assert.equal(result.patches.length, 0);
  assert.equal(await fs.readFile(path.join(fixture.paths.outDir, "LICENSE"), "utf8"), fixture.files.LICENSE);
  assert.equal(await fs.readFile(path.join(fixture.paths.outDir, "ui/litellm-dashboard/package.json"), "utf8"), fixture.files["ui/litellm-dashboard/package.json"]);
  assert.equal(await fs.readFile(path.join(fixture.paths.outDir, "ui/litellm-dashboard/package-lock.json"), "utf8"), fixture.files["ui/litellm-dashboard/package-lock.json"]);
  assert.equal(await fs.readFile(path.join(fixture.paths.outDir, "ui/litellm-dashboard/README.md"), "utf8"), fixture.files["ui/litellm-dashboard/README.md"]);
  assert.equal((await fs.stat(path.join(fixture.paths.outDir, "ui/litellm-dashboard/package.json"))).mode & 0o777, 0o664);
  assert.equal((await fs.stat(path.join(fixture.paths.outDir, "ui"))).mode & 0o777, 0o775);
  assert.ok(await exists(path.join(fixture.paths.outDir, "litellm/proxy/logo_monogram_dark.png")));
  assert.equal(await exists(path.join(fixture.paths.outDir, "ui/litellm-dashboard/_experimental/out")), false);
  assert.equal(await exists(path.join(fixture.paths.outDir, "ui/enterprise")), false);
  assert.equal(await exists(path.join(fixture.paths.outDir, "README.md")), false);
  assert.equal(await exists(fixture.paths.cacheDir), false);
});

test("prepare fetches the pinned commit into the private cache", async (t) => {
  const fixture = await createFixture(t);
  const result = await prepare({
    pin: fixture.fetchPin,
    cacheDir: fixture.paths.cacheDir,
    outDir: fixture.paths.outDir,
    patchesDir: fixture.paths.patchesDir,
  });
  assert.equal(result.commit, fixture.pin.commit);
  assert.ok(await exists(path.join(fixture.paths.cacheDir, "repo.git")));
  const cached = await fs.readFile(path.join(fixture.paths.cacheDir, "archive.tar"));
  assert.equal(hashBytes(cached), fixture.pin.archiveSha256);
  assert.equal(await fs.readFile(path.join(fixture.paths.outDir, "LICENSE"), "utf8"), fixture.files.LICENSE);
});

test("a modified working tree in the source checkout is not used", async (t) => {
  const fixture = await createFixture(t);
  const target = path.join(fixture.repo, "ui/litellm-dashboard/next.config.mjs");
  await fs.writeFile(target, "export default { output: \"local-edit\" };\n");
  await prepare({
    pin: fixture.pin,
    sourceRepo: fixture.repo,
    cacheDir: fixture.paths.cacheDir,
    outDir: fixture.paths.outDir,
    patchesDir: fixture.paths.patchesDir,
  });
  assert.equal(await fs.readFile(path.join(fixture.paths.outDir, "ui/litellm-dashboard/next.config.mjs"), "utf8"), fixture.files["ui/litellm-dashboard/next.config.mjs"]);
});

test("a corrupted cached archive fails before the next tree is published", async (t) => {
  const fixture = await createFixture(t);
  const options = {
    pin: fixture.fetchPin,
    cacheDir: fixture.paths.cacheDir,
    outDir: fixture.paths.outDir,
    patchesDir: fixture.paths.patchesDir,
  };
  await prepare(options);
  await fs.writeFile(path.join(fixture.paths.outDir, "sentinel"), "keep");
  await fs.writeFile(path.join(fixture.paths.cacheDir, "archive.tar"), "not the pinned archive");
  await rejected(prepare(options), "cache-corrupt");
  assert.equal(await fs.readFile(path.join(fixture.paths.outDir, "sentinel"), "utf8"), "keep");
  assert.equal(await exists(path.join(fixture.paths.outDir, "README.md")), false);
});

test("a corrupted object cache fails before the next tree is published", async (t) => {
  const fixture = await createFixture(t);
  const options = {
    pin: fixture.fetchPin,
    cacheDir: fixture.paths.cacheDir,
    outDir: fixture.paths.outDir,
    patchesDir: fixture.paths.patchesDir,
  };
  await prepare(options);
  await fs.rm(fixture.paths.outDir, { recursive: true, force: true });
  const objects = path.join(fixture.paths.cacheDir, "repo.git", "objects");
  const loose = [];
  for (const name of await fs.readdir(objects)) {
    if (!/^[0-9a-f]{2}$/.test(name)) continue;
    for (const file of await fs.readdir(path.join(objects, name))) {
      if (/^[0-9a-f]{38}$/.test(file)) loose.push(path.join(objects, name, file));
    }
  }
  assert.ok(loose.length > 0, "the fixture fetch stores loose objects");
  await fs.chmod(loose[0], 0o644);
  await fs.writeFile(loose[0], "corrupted object");
  await rejected(prepare(options), "cache-corrupt");
  assert.equal(await exists(fixture.paths.outDir), false);
});

test("an archive that does not match the pin fails before publishing", async (t) => {
  const fixture = await createFixture(t);
  await fs.mkdir(fixture.paths.outDir, { recursive: true });
  await fs.writeFile(path.join(fixture.paths.outDir, "sentinel"), "keep");
  const pin = { ...fixture.pin, archiveSha256: "0".repeat(64) };
  await rejected(
    prepare({ pin, sourceRepo: fixture.repo, cacheDir: fixture.paths.cacheDir, outDir: fixture.paths.outDir, patchesDir: fixture.paths.patchesDir }),
    "archive-mismatch",
  );
  assert.equal(await fs.readFile(path.join(fixture.paths.outDir, "sentinel"), "utf8"), "keep");
});

test("a lockfile that differs from the pin fails before publishing", async (t) => {
  const fixture = await createFixture(t);
  const pin = {
    ...fixture.pin,
    protectedFiles: fixture.pin.protectedFiles.map((file) =>
      file.path.endsWith("package-lock.json") ? { ...file, sha256: "1".repeat(64) } : file,
    ),
  };
  await rejected(
    prepare({ pin, sourceRepo: fixture.repo, cacheDir: fixture.paths.cacheDir, outDir: fixture.paths.outDir, patchesDir: fixture.paths.patchesDir }),
    "protected-file",
  );
  assert.equal(await exists(fixture.paths.outDir), false);
});

test("a dashboard tree that differs from the pin is rejected", async (t) => {
  const fixture = await createFixture(t);
  const pin = { ...fixture.pin, dashboardTree: "0".repeat(40) };
  await rejected(
    prepare({ pin, sourceRepo: fixture.repo, cacheDir: fixture.paths.cacheDir, outDir: fixture.paths.outDir, patchesDir: fixture.paths.patchesDir }),
    "dashboard-tree",
  );
});

test("a tag object is not accepted as the pinned commit", async (t) => {
  const fixture = await createFixture(t);
  git(fixture.repo, ["tag", "-a", "v1", "-m", "tag"]);
  const tagObject = git(fixture.repo, ["rev-parse", "v1"]);
  const pin = { ...fixture.pin, commit: tagObject };
  await rejected(
    prepare({ pin, sourceRepo: fixture.repo, cacheDir: fixture.paths.cacheDir, outDir: fixture.paths.outDir, patchesDir: fixture.paths.patchesDir }),
    "source-commit",
  );
});

test("patches apply in filename order and check their recorded hashes", async (t) => {
  const fixture = await createFixture(t);
  const target = "ui/litellm-dashboard/next.config.mjs";
  const before = fixture.files[target];
  const middle = "export default { output: \"export\" };\n";
  const after = "export default { output: \"export\", basePath: \"\" };\n";
  await writePatchPair(fixture.paths.patchesDir, "10-first.patch", singleLinePatch(target, before, middle), [
    { path: target, preimage: hashText(before), postimage: hashText(middle) },
  ]);
  await writePatchPair(fixture.paths.patchesDir, "20-second.patch", singleLinePatch(target, middle, after), [
    { path: target, preimage: hashText(middle), postimage: hashText(after) },
  ]);
  const result = await prepare({
    pin: fixture.pin,
    sourceRepo: fixture.repo,
    cacheDir: fixture.paths.cacheDir,
    outDir: fixture.paths.outDir,
    patchesDir: fixture.paths.patchesDir,
  });
  assert.deepEqual(result.patches, ["10-first.patch", "20-second.patch"]);
  assert.equal(await fs.readFile(path.join(fixture.paths.outDir, target), "utf8"), after);
});

test("patches can add and delete files recorded with a null preimage or postimage", async (t) => {
  const fixture = await createFixture(t);
  const added = "ui/litellm-dashboard/src/brand.ts";
  const addedContent = "export const brand = \"betterlite\";\n";
  const deleted = "ui/Dockerfile";
  const patch = [
    `diff --git a/${added} b/${added}`,
    "new file mode 100644",
    `--- /dev/null`,
    `+++ b/${added}`,
    "@@ -0,0 +1 @@",
    `+${addedContent.replace(/\n$/, "")}`,
    `diff --git a/${deleted} b/${deleted}`,
    "deleted file mode 100644",
    `--- a/${deleted}`,
    `+++ /dev/null`,
    "@@ -1 +0,0 @@",
    `-${fixture.files[deleted].replace(/\n$/, "")}`,
    "",
  ].join("\n");
  await writePatchPair(fixture.paths.patchesDir, "10-files.patch", patch, [
    { path: added, preimage: null, postimage: hashText(addedContent) },
    { path: deleted, preimage: hashText(fixture.files[deleted]), postimage: null },
  ]);
  await prepare({
    pin: fixture.pin,
    sourceRepo: fixture.repo,
    cacheDir: fixture.paths.cacheDir,
    outDir: fixture.paths.outDir,
    patchesDir: fixture.paths.patchesDir,
  });
  assert.equal(await fs.readFile(path.join(fixture.paths.outDir, added), "utf8"), addedContent);
  assert.equal(await exists(path.join(fixture.paths.outDir, deleted)), false);
});

test("a patch whose preimage does not match fails before publishing", async (t) => {
  const fixture = await createFixture(t);
  const target = "ui/litellm-dashboard/next.config.mjs";
  const before = fixture.files[target];
  const after = "export default { output: \"export\" };\n";
  await fs.mkdir(fixture.paths.outDir, { recursive: true });
  await fs.writeFile(path.join(fixture.paths.outDir, "sentinel"), "keep");
  await writePatchPair(fixture.paths.patchesDir, "10-first.patch", singleLinePatch(target, before, after), [
    { path: target, preimage: hashText("something else\n"), postimage: hashText(after) },
  ]);
  await rejected(
    prepare({ pin: fixture.pin, sourceRepo: fixture.repo, cacheDir: fixture.paths.cacheDir, outDir: fixture.paths.outDir, patchesDir: fixture.paths.patchesDir }),
    "patch-hash",
  );
  assert.equal(await fs.readFile(path.join(fixture.paths.outDir, "sentinel"), "utf8"), "keep");
});

test("a patch that writes outside the selected export is rejected", async (t) => {
  const fixture = await createFixture(t);
  const target = "litellm/proxy/extra.py";
  const content = "print('extra')\n";
  const patch = [
    `diff --git a/${target} b/${target}`,
    "new file mode 100644",
    "index 0000000..1111111",
    `--- /dev/null`,
    `+++ b/${target}`,
    "@@ -0,0 +1 @@",
    `+${content.replace(/\n$/, "")}`,
    "",
  ].join("\n");
  await writePatchPair(fixture.paths.patchesDir, "10-extra.patch", patch, [
    { path: target, preimage: null, postimage: hashText(content) },
  ]);
  await fs.mkdir(fixture.paths.outDir, { recursive: true });
  await fs.writeFile(path.join(fixture.paths.outDir, "sentinel"), "keep");
  await rejected(
    prepare({ pin: fixture.pin, sourceRepo: fixture.repo, cacheDir: fixture.paths.cacheDir, outDir: fixture.paths.outDir, patchesDir: fixture.paths.patchesDir }),
    "patch-path",
  );
  assert.equal(await fs.readFile(path.join(fixture.paths.outDir, "sentinel"), "utf8"), "keep");
  assert.equal(await exists(path.join(fixture.paths.outDir, "ui/litellm-dashboard/next.config.mjs")), false);
});

test("a patch that changes a pinned dependency file is rejected", async (t) => {
  const fixture = await createFixture(t);
  const target = "ui/litellm-dashboard/package-lock.json";
  const before = fixture.files[target];
  const after = before.replace("3", "4");
  const patch = [
    `diff --git a/${target} b/${target}`,
    `--- a/${target}`,
    `+++ b/${target}`,
    "@@ -1,3 +1,3 @@",
    " {",
    "-  \"lockfileVersion\": 3",
    "+  \"lockfileVersion\": 4",
    " }",
    "",
  ].join("\n");
  await writePatchPair(fixture.paths.patchesDir, "10-lock.patch", patch, [
    { path: target, preimage: hashText(before), postimage: hashText(after) },
  ]);
  await rejected(
    prepare({ pin: fixture.pin, sourceRepo: fixture.repo, cacheDir: fixture.paths.cacheDir, outDir: fixture.paths.outDir, patchesDir: fixture.paths.patchesDir }),
    "patch-protected",
  );
});

test("a patch whose +++ header names another path is rejected", async (t) => {
  const fixture = await createFixture(t);
  const listed = "ui/litellm-dashboard/next.config.mjs";
  const smuggled = "litellm/proxy/extra.py";
  const patch = [
    `diff --git a/${listed} b/${listed}`,
    `--- a/${listed}`,
    `+++ b/${smuggled}`,
    "@@ -1 +1 @@",
    `-${fixture.files[listed].replace(/\n$/, "")}`,
    "+export default {};",
    "",
  ].join("\n");
  await writePatchPair(fixture.paths.patchesDir, "10-smuggle.patch", patch, [
    { path: listed, preimage: hashText(fixture.files[listed]), postimage: hashText("export default {};\n") },
  ]);
  await rejected(
    prepare({ pin: fixture.pin, sourceRepo: fixture.repo, cacheDir: fixture.paths.cacheDir, outDir: fixture.paths.outDir, patchesDir: fixture.paths.patchesDir }),
    "patch-form",
  );
});

test("a patch that touches a file its record does not list is rejected", async (t) => {
  const fixture = await createFixture(t);
  const listed = "ui/litellm-dashboard/next.config.mjs";
  const unlisted = "ui/litellm-dashboard/src/app/page.tsx";
  const patch = [
    singleLinePatch(listed, fixture.files[listed], "export default {};\n").trimEnd(),
    `diff --git a/${unlisted} b/${unlisted}`,
    `--- a/${unlisted}`,
    `+++ b/${unlisted}`,
    "@@ -1,3 +1,3 @@",
    " export default function Page() {",
    "-  return null;",
    "+  return false;",
    " }",
    "",
  ].join("\n");
  await writePatchPair(fixture.paths.patchesDir, "10-two.patch", patch, [
    { path: listed, preimage: hashText(fixture.files[listed]), postimage: hashText("export default {};\n") },
  ]);
  await rejected(
    prepare({ pin: fixture.pin, sourceRepo: fixture.repo, cacheDir: fixture.paths.cacheDir, outDir: fixture.paths.outDir, patchesDir: fixture.paths.patchesDir }),
    "patch-record",
  );
});

test("a patch without its adjacent record is rejected", async (t) => {
  const fixture = await createFixture(t);
  const target = "ui/litellm-dashboard/next.config.mjs";
  const before = fixture.files[target];
  await fs.mkdir(fixture.paths.patchesDir, { recursive: true });
  await fs.writeFile(path.join(fixture.paths.patchesDir, "10-lonely.patch"), singleLinePatch(target, before, "export default {};\n"));
  await rejected(
    prepare({ pin: fixture.pin, sourceRepo: fixture.repo, cacheDir: fixture.paths.cacheDir, outDir: fixture.paths.outDir, patchesDir: fixture.paths.patchesDir }),
    "patch-record",
  );
});

test("git subprocesses time out and their process group is killed", async (t) => {
  const fixture = await createFixture(t);
  const fake = path.join(fixture.base, "fake-git");
  const pidFile = path.join(fixture.base, "fake-git.pid");
  await fs.mkdir(fake);
  await fs.writeFile(path.join(fake, "git"), "#!/bin/sh\nprintf '%s' \"$$\" > \"$FAKE_GIT_PID_FILE\"\nexec sleep 60\n", { mode: 0o755 });
  const originalPath = process.env.PATH;
  process.env.PATH = `${fake}${path.delimiter}${originalPath}`;
  process.env.FAKE_GIT_PID_FILE = pidFile;
  try {
    await rejected(
      prepare({ pin: fixture.pin, sourceRepo: fixture.repo, cacheDir: fixture.paths.cacheDir, outDir: fixture.paths.outDir, patchesDir: fixture.paths.patchesDir, timeoutMs: 150 }),
      "timeout",
    );
  } finally {
    process.env.PATH = originalPath;
    delete process.env.FAKE_GIT_PID_FILE;
  }
  let pid = null;
  for (let attempt = 0; attempt < 100 && pid === null; attempt += 1) {
    try {
      pid = Number(await fs.readFile(pidFile, "utf8"));
    } catch {
      await new Promise((resolve) => setTimeout(resolve, 10));
    }
  }
  assert.ok(Number.isSafeInteger(pid) && pid > 0, "the fake git recorded its process id");
  let gone = false;
  for (let attempt = 0; attempt < 100 && !gone; attempt += 1) {
    try {
      process.kill(pid, 0);
      await new Promise((resolve) => setTimeout(resolve, 20));
    } catch (error) {
      if (error.code !== "ESRCH") throw error;
      gone = true;
    }
  }
  assert.ok(gone, "the timed out process group is gone");
});

test("verify --repeat prepares two matching trees through the command line", async (t) => {
  const fixture = await createFixture(t);
  await fs.writeFile(fixture.paths.pinPath, `${JSON.stringify(fixture.pin, null, 2)}\n`);
  const output = execFileSync(
    process.execPath,
    [
      SOURCE, "verify", "--repeat",
      "--pin", fixture.paths.pinPath,
      "--source-repo", fixture.repo,
      "--cache", fixture.paths.cacheDir,
      "--patches", fixture.paths.patchesDir,
    ],
    { encoding: "utf8", env: { ...process.env } },
  );
  assert.match(output, /verified two preparations of [0-9a-f]{40} match/);
  assert.deepEqual(await fs.readdir(path.join(fixture.base, "verify")), []);
});

test("compareTrees reports a difference in content, mode or presence", async (t) => {
  const base = await fs.mkdtemp(path.join(os.tmpdir(), "b9090-compare-"));
  t.after(() => fs.rm(base, { recursive: true, force: true }));
  const first = path.join(base, "first");
  const second = path.join(base, "second");
  for (const directory of [first, second]) {
    await fs.mkdir(directory, { recursive: true });
    await fs.writeFile(path.join(directory, "a.txt"), "same\n");
    await fs.chmod(path.join(directory, "a.txt"), 0o664);
  }
  assert.equal(await compareTrees(first, second), 1);
  for (const [change, restore] of [
    [() => fs.writeFile(path.join(second, "a.txt"), "different\n"), () => fs.writeFile(path.join(second, "a.txt"), "same\n")],
    [() => fs.chmod(path.join(second, "a.txt"), 0o600), () => fs.chmod(path.join(second, "a.txt"), 0o664)],
    [() => fs.writeFile(path.join(second, "b.txt"), "same\n"), () => fs.rm(path.join(second, "b.txt"))],
  ]) {
    await change();
    await rejected(compareTrees(first, second), "reproducibility");
    await restore();
  }
  assert.equal(await compareTrees(first, second), 1);
});

test("verifyRepeat compares the two prepared trees", async (t) => {
  const fixture = await createFixture(t);
  const result = await verifyRepeat({
    pin: fixture.pin,
    sourceRepo: fixture.repo,
    cacheDir: fixture.paths.cacheDir,
    patchesDir: fixture.paths.patchesDir,
  });
  assert.equal(result.commit, fixture.pin.commit);
  assert.ok(result.entries > 0);
});
