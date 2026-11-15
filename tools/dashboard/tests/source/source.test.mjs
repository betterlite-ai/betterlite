import assert from "node:assert/strict";
import { execFileSync, spawn } from "node:child_process";
import { existsSync } from "node:fs";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { test } from "node:test";

import { fileURLToPath } from "node:url";

import { listPatches, readPatchFiles } from "../../patches.mjs";
import { compareTrees, verifyRepeat } from "../../source/cli.mjs";
import { prepare } from "../../source.mjs";
import { createFixture, exists, git, hashBytes, hashText, singleLinePatch, writePatchPair } from "./fixture.mjs";

const SOURCE = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../../source.mjs");

async function rejected(promise, code) {
  await assert.rejects(promise, (error) => {
    assert.equal(error.code, code, error.message);
    return true;
  });
}

async function waitForPid(file) {
  for (let attempt = 0; attempt < 200; attempt += 1) {
    const value = await fs.readFile(file, "utf8").then((text) => Number(text.trim()), () => 0);
    if (Number.isSafeInteger(value) && value > 0) return value;
    await new Promise((resolve) => setTimeout(resolve, 10));
  }
  throw new Error("the fake git did not record a child pid");
}

async function alive(pid) {
  try {
    process.kill(pid, 0);
  } catch (error) {
    if (error.code === "ESRCH") return false;
    throw error;
  }
  const stat = await fs.readFile(`/proc/${pid}/stat`, "utf8").catch(() => null);
  if (stat === null) return true;
  const close = stat.lastIndexOf(")");
  return stat.slice(close + 2, close + 3) !== "Z";
}

async function waitForGone(pid) {
  for (let attempt = 0; attempt < 100; attempt += 1) {
    if (!(await alive(pid))) return true;
    await new Promise((resolve) => setTimeout(resolve, 20));
  }
  return false;
}

function killQuietly(pid) {
  try {
    process.kill(pid, "SIGKILL");
  } catch {
    return;
  }
}

function realGitPath() {
  for (const directory of (process.env.PATH ?? "").split(path.delimiter)) {
    const candidate = path.join(directory, "git");
    if (existsSync(candidate)) return candidate;
  }
  throw new Error("no git on PATH");
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

test("hostile inherited git configuration cannot change the archive", async (t) => {
  const fixture = await createFixture(t);
  const home = path.join(fixture.base, "home");
  await fs.mkdir(home, { recursive: true });
  await fs.writeFile(path.join(home, ".gitconfig"), "[core]\n\tautocrlf = true\n");
  const hostile = {
    HOME: home,
    GIT_DIR: path.join(fixture.base, "not-a-repository"),
    GIT_CONFIG_COUNT: "1",
    GIT_CONFIG_KEY_0: "core.autocrlf",
    GIT_CONFIG_VALUE_0: "true",
    GIT_CONFIG_PARAMETERS: "'core.autocrlf'='true'",
  };
  const saved = new Map([["GIT_CONFIG_GLOBAL", process.env.GIT_CONFIG_GLOBAL]]);
  for (const [name, value] of Object.entries(hostile)) {
    saved.set(name, process.env[name]);
    process.env[name] = value;
  }
  delete process.env.GIT_CONFIG_GLOBAL;
  try {
    const result = await prepare({
      pin: fixture.pin,
      sourceRepo: fixture.repo,
      cacheDir: fixture.paths.cacheDir,
      outDir: fixture.paths.outDir,
      patchesDir: fixture.paths.patchesDir,
    });
    assert.equal(result.archiveSha256, fixture.pin.archiveSha256);
    assert.equal(await fs.readFile(path.join(fixture.paths.outDir, "LICENSE"), "utf8"), fixture.files.LICENSE);
  } finally {
    for (const [name, value] of saved) {
      if (value === undefined) delete process.env[name];
      else process.env[name] = value;
    }
  }
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

test("a patch whose postimage does not match is rejected after applying", async (t) => {
  const fixture = await createFixture(t);
  const target = "ui/litellm-dashboard/next.config.mjs";
  const before = fixture.files[target];
  const after = "export default { output: \"export\" };\n";
  await fs.mkdir(fixture.paths.outDir, { recursive: true });
  await fs.writeFile(path.join(fixture.paths.outDir, "sentinel"), "keep");
  await writePatchPair(fixture.paths.patchesDir, "10-first.patch", singleLinePatch(target, before, after), [
    { path: target, preimage: hashText(before), postimage: hashText("something else\n") },
  ]);
  await rejected(
    prepare({ pin: fixture.pin, sourceRepo: fixture.repo, cacheDir: fixture.paths.cacheDir, outDir: fixture.paths.outDir, patchesDir: fixture.paths.patchesDir }),
    "patch-hash",
  );
  assert.equal(await fs.readFile(path.join(fixture.paths.outDir, "sentinel"), "utf8"), "keep");
  assert.equal(await exists(path.join(fixture.paths.outDir, target)), false);
});

test("a patch whose hunk does not apply fails the git apply check", async (t) => {
  const fixture = await createFixture(t);
  const target = "ui/litellm-dashboard/next.config.mjs";
  const before = fixture.files[target];
  const after = "export default { output: \"export\" };\n";
  await writePatchPair(fixture.paths.patchesDir, "10-badhunk.patch", singleLinePatch(target, "export default { output: \"other\" };\n", after), [
    { path: target, preimage: hashText(before), postimage: hashText(after) },
  ]);
  await fs.mkdir(fixture.paths.outDir, { recursive: true });
  await fs.writeFile(path.join(fixture.paths.outDir, "sentinel"), "keep");
  await rejected(
    prepare({ pin: fixture.pin, sourceRepo: fixture.repo, cacheDir: fixture.paths.cacheDir, outDir: fixture.paths.outDir, patchesDir: fixture.paths.patchesDir }),
    "patch-check",
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

test("a patch whose file headers name another path than its diff is rejected", async (t) => {
  const fixture = await createFixture(t);
  const listed = "ui/litellm-dashboard/next.config.mjs";
  const applied = "ui/litellm-dashboard/src/app/page.tsx";
  const patch = [
    `diff --git a/${listed} b/${listed}`,
    `--- a/${applied}`,
    `+++ b/${applied}`,
    "@@ -1,3 +1,3 @@",
    " export default function Page() {",
    "-  return null;",
    "+  return false;",
    " }",
    "",
  ].join("\n");
  await writePatchPair(fixture.paths.patchesDir, "10-smuggle.patch", patch, [
    { path: listed, preimage: hashText(fixture.files[listed]), postimage: hashText("export default {};\n") },
  ]);
  await rejected(
    prepare({ pin: fixture.pin, sourceRepo: fixture.repo, cacheDir: fixture.paths.cacheDir, outDir: fixture.paths.outDir, patchesDir: fixture.paths.patchesDir }),
    "patch-record",
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

test("the patch listing holds only patch files and an absent directory is empty", async (t) => {
  const fixture = await createFixture(t);
  assert.deepEqual(await listPatches(fixture.paths.patchesDir), []);
  await writePatchPair(fixture.paths.patchesDir, "20-b.patch", "second\n", []);
  await writePatchPair(fixture.paths.patchesDir, "10-a.patch", "first\n", []);
  await fs.writeFile(path.join(fixture.paths.patchesDir, "notes.txt"), "ignored\n");
  await fs.mkdir(path.join(fixture.paths.patchesDir, "30-dir.patch"));
  assert.deepEqual(await listPatches(fixture.paths.patchesDir), ["10-a.patch", "20-b.patch"]);
});

test("patch files come back with their raw bytes and parsed targets", async (t) => {
  const fixture = await createFixture(t);
  const target = "ui/litellm-dashboard/next.config.mjs";
  const targets = [{ path: target, preimage: hashText(fixture.files[target]), postimage: hashText("export default {};\n") }];
  const patchText = singleLinePatch(target, fixture.files[target], "export default {};\n");
  await writePatchPair(fixture.paths.patchesDir, "10-first.patch", patchText, targets);
  const files = await readPatchFiles(fixture.paths.patchesDir);
  assert.equal(files.length, 1);
  assert.equal(files[0].name, "10-first.patch");
  assert.equal(files[0].patch.toString("utf8"), patchText);
  assert.equal(files[0].record.toString("utf8"), `${JSON.stringify({ targets }, null, 2)}\n`);
  assert.deepEqual(files[0].targets, targets);
  await fs.rm(path.join(fixture.paths.patchesDir, "10-first.json"));
  await assert.rejects(
    readPatchFiles(fixture.paths.patchesDir),
    (error) => error.code === "patch-record" && /10-first\.json record/.test(error.message),
  );
});

const FAKE_GIT = "#!/bin/sh\nsleep 60 &\nprintf '%s' \"$!\" > \"$FAKE_GIT_PID_FILE\"\nwait\n";

test("git subprocesses time out and kill their whole process group", async (t) => {
  const fixture = await createFixture(t);
  const fake = path.join(fixture.base, "fake-git");
  const pidFile = path.join(fixture.base, "fake-git.pid");
  await fs.mkdir(fake);
  await fs.writeFile(path.join(fake, "git"), FAKE_GIT, { mode: 0o755 });
  const originalPath = process.env.PATH;
  process.env.PATH = `${fake}${path.delimiter}${originalPath}`;
  process.env.FAKE_GIT_PID_FILE = pidFile;
  try {
    await rejected(
      prepare({ pin: fixture.pin, sourceRepo: fixture.repo, cacheDir: fixture.paths.cacheDir, outDir: fixture.paths.outDir, patchesDir: fixture.paths.patchesDir, timeoutMs: 1000 }),
      "timeout",
    );
  } finally {
    process.env.PATH = originalPath;
    delete process.env.FAKE_GIT_PID_FILE;
  }
  const pid = await waitForPid(pidFile);
  t.after(() => killQuietly(pid));
  assert.equal(await waitForGone(pid), true, "the grandchild of the timed out git is gone");
});

test("SIGTERM kills the git process group and leaves no published tree", async (t) => {
  const fixture = await createFixture(t);
  await fs.writeFile(fixture.paths.pinPath, `${JSON.stringify(fixture.pin, null, 2)}\n`);
  const fake = path.join(fixture.base, "fake-git");
  const pidFile = path.join(fixture.base, "cli-git.pid");
  await fs.mkdir(fake);
  await fs.writeFile(path.join(fake, "git"), FAKE_GIT, { mode: 0o755 });
  const child = spawn(
    process.execPath,
    [
      SOURCE, "prepare",
      "--pin", fixture.paths.pinPath,
      "--source-repo", fixture.repo,
      "--cache", fixture.paths.cacheDir,
      "--out", fixture.paths.outDir,
      "--patches", fixture.paths.patchesDir,
    ],
    {
      env: { ...process.env, PATH: `${fake}${path.delimiter}${process.env.PATH}`, FAKE_GIT_PID_FILE: pidFile },
      stdio: "ignore",
    },
  );
  const exited = new Promise((resolve) => child.on("exit", (code, signal) => resolve({ code, signal })));
  const pid = await waitForPid(pidFile);
  t.after(() => {
    killQuietly(pid);
    child.kill("SIGKILL");
  });
  child.kill("SIGTERM");
  const { code, signal } = await exited;
  assert.equal(code, 143);
  assert.equal(signal, null);
  assert.equal(await waitForGone(pid), true, "the cancelled git grandchild is gone");
  assert.equal(await exists(fixture.paths.outDir), false);
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
  assert.equal((await compareTrees(first, second)).length, 1);
  for (const [change, restore] of [
    [() => fs.writeFile(path.join(second, "a.txt"), "different\n"), () => fs.writeFile(path.join(second, "a.txt"), "same\n")],
    [() => fs.chmod(path.join(second, "a.txt"), 0o600), () => fs.chmod(path.join(second, "a.txt"), 0o664)],
    [() => fs.writeFile(path.join(second, "b.txt"), "same\n"), () => fs.rm(path.join(second, "b.txt"))],
  ]) {
    await change();
    await rejected(compareTrees(first, second), "reproducibility");
    await restore();
  }
  assert.equal((await compareTrees(first, second)).length, 1);
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
  assert.equal(result.comparedPaths.length, result.entries);
  assert.ok(result.comparedPaths.includes("LICENSE"));
  assert.ok(result.comparedPaths.includes("ui/litellm-dashboard/package-lock.json"));
  assert.equal(result.comparedPaths.includes("ui/litellm-dashboard/_experimental/out/index.html"), false);
});

test("verifyRepeat rejects two preparations that differ", async (t) => {
  const fixture = await createFixture(t);
  const target = "ui/litellm-dashboard/next.config.mjs";
  const before = fixture.files[target];
  const after = "export default { output: \"export\" };\n";
  await writePatchPair(fixture.paths.patchesDir, "10-first.patch", singleLinePatch(target, before, after), [
    { path: target, preimage: hashText(before), postimage: hashText(after) },
  ]);
  const wrapper = path.join(fixture.base, "wrapper-git");
  const countFile = path.join(fixture.base, "apply-count");
  const real = realGitPath();
  await fs.mkdir(wrapper);
  await fs.writeFile(
    path.join(wrapper, "git"),
    [
      "#!/bin/sh",
      `"${real}" "$@"`,
      "status=$?",
      "if [ $status -ne 0 ]; then exit $status; fi",
      'case " $* " in',
      '  *" --check "*) exit 0 ;;',
      "esac",
      'if [ "$1" = "apply" ]; then',
      '  count=$(cat "$WRAPPER_COUNT_FILE" 2>/dev/null || echo 0)',
      "  count=$((count + 1))",
      '  echo "$count" > "$WRAPPER_COUNT_FILE"',
      '  if [ "$count" -eq 2 ]; then printf "extra\\n" > extra.txt; fi',
      "fi",
      "exit 0",
      "",
    ].join("\n"),
    { mode: 0o755 },
  );
  const originalPath = process.env.PATH;
  process.env.PATH = `${wrapper}${path.delimiter}${originalPath}`;
  process.env.WRAPPER_COUNT_FILE = countFile;
  try {
    await rejected(
      verifyRepeat({ pin: fixture.pin, sourceRepo: fixture.repo, cacheDir: fixture.paths.cacheDir, patchesDir: fixture.paths.patchesDir }),
      "reproducibility",
    );
  } finally {
    process.env.PATH = originalPath;
    delete process.env.WRAPPER_COUNT_FILE;
  }
});
