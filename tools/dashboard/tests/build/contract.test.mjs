import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs/promises";
import path from "node:path";
import test from "node:test";

import { REPO_ROOT, put, scratch } from "./fixtures.mjs";

const BUILD_SH = path.join(REPO_ROOT, "tools/dashboard/build.sh");

async function dockerfile() {
  return fs.readFile(path.join(REPO_ROOT, "packaging/ui/Dockerfile"), "utf8");
}

function stage(text, name) {
  const start = text.search(new RegExp(`^FROM .* AS ${name}$`, "m"));
  assert.notEqual(start, -1, `stage ${name} exists`);
  const rest = text.slice(start + 1);
  const next = rest.search(/^FROM /m);
  return next === -1 ? text.slice(start) : text.slice(start, start + 1 + next);
}

test("the builder is the digest-pinned node 24.19 alpine 3.24 image", async () => {
  const text = await dockerfile();
  assert.match(
    text,
    /^ARG UI_BUILD_IMAGE=node:24\.19-alpine3\.24@sha256:d32cdf619f63fe0471182d08996dd516c6275bb5fd31ae06e55a570bd9e1ad43$/m,
  );
  for (const from of text.match(/^FROM .*$/gm)) {
    assert.match(from, /^FROM (--platform=\$BUILDPLATFORM \$UI_BUILD_IMAGE|\w+|scratch) AS \w+$/, from);
  }
});

test("dependencies install from the original lock and npmrc on the build platform", async () => {
  const text = stage(await dockerfile(), "dependencies");
  assert.match(text, /^FROM --platform=\$BUILDPLATFORM \$UI_BUILD_IMAGE AS dependencies$/m);
  assert.match(text, /^WORKDIR \/work\/dashboard$/m);
  assert.match(text, /^COPY ui\/litellm-dashboard\/package\.json ui\/litellm-dashboard\/package-lock\.json ui\/litellm-dashboard\/\.npmrc \.\/$/m);
  assert.match(text, /npm ci --prefer-offline$/m);
  assert.match(text, /^ENV NEXT_TELEMETRY_DISABLED=1 \\\n\s+npm_config_fund=false \\\n\s+npm_config_audit=false$/m);
  assert.equal(text.indexOf("COPY ui/litellm-dashboard/package.json") < text.indexOf("npm ci"), true);
});

test("the application sources arrive after the dependencies", async () => {
  const text = await dockerfile();
  assert.match(stage(text, "application"), /^FROM dependencies AS application\nCOPY ui\/litellm-dashboard\/ \.\/$/m);
});

test("the compile has no network and builds for production with the scrubbed environment", async () => {
  const text = stage(await dockerfile(), "compiled");
  assert.match(text, /^RUN --network=none <<'SCRIPT'$/m);
  assert.match(text, /^unset NEXT_PUBLIC_BASE_URL NEXT_PUBLIC_USE_REWRITES$/m);
  assert.match(text, /^  export NEXT_PUBLIC_BASE_URL="\$API_BASE"$/m);
  assert.match(text, /NODE_ENV=production BETTERLITE_BUILD_ID="\$\{BUILD_ID:\?\}" npm run build$/m);
  assert.match(text, /^ARG API_BASE=""$/m);
  assert.equal(text.includes("npm ci"), false, "the compile stage never installs");
});

test("the bundling step has no network and the export holds only /bundle", async () => {
  const text = await dockerfile();
  assert.match(stage(text, "bundled"), /^RUN --network=none node \/opt\/bundle\.mjs finalize/m);
  assert.equal(stage(text, "export"), "FROM scratch AS export\nCOPY --from=bundled /bundle /bundle\n\n");
});

test("the image copies the modules bundle.mjs imports beside it", async () => {
  const text = await dockerfile();
  for (const name of ["bundled", "checks"]) {
    const copy = stage(text, name);
    assert.match(copy, /^COPY --from=tools bundle\.mjs patches\.mjs \/opt\/$/m, name);
    assert.match(copy, /^COPY --from=tools bundle \/opt\/bundle$/m, name);
  }
});

test("the checks run the shared entrypoint without network", async () => {
  const text = stage(await dockerfile(), "checks");
  assert.match(text, /^COPY --from=checkplan tests\/ tests\/betterlite\/$/m);
  assert.match(text, /^RUN --network=none node \/opt\/bundle\.mjs check --plan \/opt\/plan\.json --app \/work\/dashboard$/m);
});

test("no secret or token has a place in the Dockerfile", async () => {
  const text = await dockerfile();
  for (const word of ["ACTIONS_RUNTIME_TOKEN", "ACTIONS_RESULTS_URL", "LITELLM_MASTER_KEY", "DATABASE_URL"]) {
    assert.equal(text.includes(word), false, word);
  }
});

function runBuildSh(args, { env = {}, path: extraPath = "" } = {}) {
  return spawnSync("bash", [BUILD_SH, ...args], {
    encoding: "utf8",
    env: { PATH: `${extraPath}${extraPath === "" ? "" : ":"}${process.env.PATH}`, HOME: process.env.HOME, ...env },
  });
}

test("build.sh key prints the key and ignores the base path and public URL variables", () => {
  const plain = runBuildSh(["key"]);
  assert.equal(plain.status, 0, plain.stderr);
  assert.match(plain.stdout, /^[0-9a-f]{64}\n$/);
  assert.equal(plain.stderr, "");
  const noisy = runBuildSh(["key"], {
    env: { UI_BASE_PATH: "/ignored", NEXT_PUBLIC_BASE_URL: "https://leak.test", NEXT_PUBLIC_USE_REWRITES: "true" },
  });
  assert.equal(noisy.stdout, plain.stdout);
  assert.match(noisy.stderr, /UI_BASE_PATH is ignored/);
  const flag = runBuildSh(["key", "--ui-base-path", "/ignored"]);
  assert.equal(flag.stdout, plain.stdout);
  assert.match(flag.stderr, /--ui-base-path is ignored/);
});

test("build.sh key follows --api-base", () => {
  const plain = runBuildSh(["key"]).stdout;
  const based = runBuildSh(["key", "--api-base", "https://api.example.test"]);
  assert.equal(based.status, 0, based.stderr);
  assert.notEqual(based.stdout, plain);
  const bad = runBuildSh(["key", "--api-base", "https://user:pw@example.test"]);
  assert.notEqual(bad.status, 0);
  assert.match(bad.stderr, /without credentials/);
});

test("build.sh refuses a call it does not understand", () => {
  for (const args of [[], ["publish"], ["key", "--nope"], ["key", "--api-base"], ["check"], ["build", "extra"]]) {
    const result = runBuildSh(args);
    assert.equal(result.status, 1, args.join(" "));
    assert.match(result.stderr, /build\.sh: /, args.join(" "));
  }
});

async function stubDocker(t) {
  const root = await scratch(t);
  const bin = path.join(root, "bin");
  await put(bin, "docker", `#!/bin/sh\necho "$*" >> "${root}/docker.log"\nexit 99\n`);
  await fs.chmod(path.join(bin, "docker"), 0o755);
  return { root, bin };
}

test("a bundle whose manifest has the current key is reused without Docker", async (t) => {
  const { root, bin } = await stubDocker(t);
  const key = runBuildSh(["key"]).stdout.trim();
  const out = path.join(root, "work");
  await put(out, "dist/ui/.litellm_ui_ready", "");
  await put(out, "dist/manifest.json", `{\n  "key": "${key}",\n  "files": 1\n}\n`);
  const result = runBuildSh(["build", "--out", out], { path: bin });
  assert.equal(result.status, 0, result.stderr);
  assert.equal(result.stdout, `dashboard bundle ${key} is current\n`);
  await assert.rejects(fs.stat(path.join(root, "docker.log")));
});

test("a bundle that is stale, unfinished or forced is not reused", async (t) => {
  const key = runBuildSh(["key"]).stdout.trim();
  const manifests = {
    stale: { manifest: `{"key": "${"0".repeat(64)}"}`, marker: true, args: [] },
    unfinished: { manifest: `{"key": "${key}"}`, marker: false, args: [] },
    forced: { manifest: `{"key": "${key}"}`, marker: true, args: ["--rebuild"] },
  };
  for (const [name, { manifest, marker, args }] of Object.entries(manifests)) {
    const { root, bin } = await stubDocker(t);
    const out = path.join(root, "work");
    await put(out, "dist/manifest.json", manifest);
    if (marker) await put(out, "dist/ui/.litellm_ui_ready", "");
    const result = runBuildSh(["build", "--out", out, "--source-repo", path.join(root, "no-such-checkout"), ...args], { path: bin });
    assert.notEqual(result.status, 0, name);
    assert.equal(result.stdout.includes("is current"), false, name);
  }
});

async function addedLines(name) {
  const patch = await fs.readFile(path.join(REPO_ROOT, "ui/dashboard/patches", name), "utf8");
  return patch.split("\n").filter((line) => line.startsWith("+") && !line.startsWith("+++"));
}

test("the build patch adds a deterministic build id and leaves the export settings alone", async () => {
  const patch = await fs.readFile(path.join(REPO_ROOT, "ui/dashboard/patches/10-build.patch"), "utf8");
  const added = (await addedLines("10-build.patch")).join("\n");
  assert.match(added, /generateBuildId: async \(\) => process\.env\.BETTERLITE_BUILD_ID \?\? null,/);
  for (const setting of ["assetPrefix", "basePath", "trailingSlash", "output", "removeConsole", "images", "unoptimized"]) {
    assert.equal(added.includes(setting), false, `${setting} keeps its upstream value`);
    assert.equal(new RegExp(`^-.*${setting}`, "m").test(patch), false, `${setting} is not removed`);
  }
});

test("the build patch swaps the Inter font for the system stack", async () => {
  const patch = await fs.readFile(path.join(REPO_ROOT, "ui/dashboard/patches/10-build.patch"), "utf8");
  assert.match(patch, /^-import \{ Inter \} from "next\/font\/google";$/m);
  assert.match(patch, /^-const inter = Inter\(\{ subsets: \["latin"\] \}\);$/m);
  assert.match(patch, /^-      <body className=\{inter\.className\}>$/m);
  assert.match(patch, /^\+      <body>$/m);
  assert.equal((await addedLines("10-build.patch")).join("\n").includes("next/font"), false);
});

test("the build patch is recorded with the hashes of the files it changes", async () => {
  const record = JSON.parse(await fs.readFile(path.join(REPO_ROOT, "ui/dashboard/patches/10-build.json"), "utf8"));
  assert.deepEqual(
    record.targets.map((target) => target.path),
    ["ui/litellm-dashboard/next.config.mjs", "ui/litellm-dashboard/src/app/layout.tsx"],
  );
  for (const target of record.targets) {
    assert.match(target.preimage, /^[0-9a-f]{64}$/);
    assert.match(target.postimage, /^[0-9a-f]{64}$/);
    assert.notEqual(target.preimage, target.postimage);
  }
});

test("the build patch adds no trailing whitespace", async () => {
  const patch = await fs.readFile(path.join(REPO_ROOT, "ui/dashboard/patches/10-build.patch"), "utf8");
  const trailing = patch.split("\n").filter((line) => line !== line.trimEnd());
  assert.deepEqual(trailing, []);
});

test("the build never commits or stages anything", async () => {
  for (const file of [
    "packaging/ui/Dockerfile",
    "tools/dashboard/build.sh",
    "tools/dashboard/bundle.mjs",
    "tools/dashboard/bundle/checks.mjs",
    "tools/dashboard/bundle/finalize.mjs",
    "tools/dashboard/bundle/inputs.mjs",
    "tools/dashboard/install.mjs",
    "tools/dashboard/patches.mjs",
  ]) {
    const text = await fs.readFile(path.join(REPO_ROOT, file), "utf8");
    assert.equal(/git (add|commit|push)/.test(text), false, file);
  }
});

test("the check installs a docker client when the task container has none", async () => {
  const text = await fs.readFile(path.join(REPO_ROOT, "tools/dashboard/check-build.sh"), "utf8");
  assert.match(text, /if ! command -v docker > \/dev\/null; then/);
  assert.match(text, /apt-get install --yes --no-install-recommends docker-cli docker-buildx/);
});
