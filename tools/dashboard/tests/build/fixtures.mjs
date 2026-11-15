import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

export const REPO_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../../../..");
export const PINNED_IMAGE = `node:24.19-alpine3.24@sha256:${"d".repeat(64)}`;

export async function scratch(t) {
  const directory = await fs.mkdtemp(path.join(os.tmpdir(), "bundle-test-"));
  t.after(() => fs.rm(directory, { recursive: true, force: true }));
  return directory;
}

export async function put(root, relative, content = "") {
  const file = path.join(root, relative);
  await fs.mkdir(path.dirname(file), { recursive: true });
  await fs.writeFile(file, content);
  return file;
}

// The files bundleInputs reads, with small stand-in contents
export async function writeRepo(root, overrides = {}) {
  const pin = {
    commit: "c".repeat(40),
    archiveSha256: "a".repeat(64),
    protectedFiles: [
      { path: "ui/litellm-dashboard/package-lock.json", sha256: "1".repeat(64) },
      { path: "ui/litellm-dashboard/.npmrc", sha256: "2".repeat(64) },
    ],
  };
  const files = {
    "tools/dashboard/source.pin.json": JSON.stringify(pin),
    "tools/dashboard/build.sh": "#!/usr/bin/env bash\n",
    "tools/dashboard/bundle.mjs": "// bundle\n",
    "tools/dashboard/patches.mjs": "// patches\n",
    "tools/dashboard/bundle/inputs.mjs": "// inputs\n",
    "tools/dashboard/bundle/checks.mjs": "// checks\n",
    "tools/dashboard/bundle/finalize.mjs": "// finalize\n",
    "packaging/ui/Dockerfile": `ARG UI_BUILD_IMAGE=${PINNED_IMAGE}\nFROM $UI_BUILD_IMAGE\n`,
    "ui/dashboard/patches/10-build.patch": "patch\n",
    "ui/dashboard/patches/10-build.json": JSON.stringify({
      targets: [
        { path: "ui/litellm-dashboard/next.config.mjs", preimage: "0".repeat(64), postimage: "1".repeat(64) },
        { path: "ui/litellm-dashboard/src/app/layout.tsx", preimage: "0".repeat(64), postimage: "1".repeat(64) },
        { path: "ui/litellm-dashboard/src/gone.tsx", preimage: "0".repeat(64), postimage: null },
        { path: "ui/litellm-dashboard/src/data.json", preimage: "0".repeat(64), postimage: "1".repeat(64) },
      ],
    }),
    ...overrides,
  };
  for (const [relative, content] of Object.entries(files)) await put(root, relative, content);
}

// An application tree and the export a Next build of it would leave
export async function writeApp(root, { license = true } = {}) {
  await put(root, "package-lock.json", JSON.stringify({
    packages: {
      "": { name: "app" },
      "node_modules/runtime-lib": { version: "1.2.3", license: "MIT" },
      "node_modules/runtime-lib/node_modules/nested-lib": { version: "0.1.0" },
      "node_modules/dev-tool": { version: "9.9.9", license: "ISC", dev: true },
      "node_modules/linked": { link: true },
      "node_modules/no-license-file": { version: "1.0.0", license: "MIT" },
    },
  }));
  if (license) {
    await put(root, "node_modules/runtime-lib/LICENSE", "runtime-lib licence text\n\n");
    await put(root, "node_modules/runtime-lib/node_modules/nested-lib/COPYING.md", "nested terms\n");
    await put(root, "node_modules/runtime-lib/README.md", "not a licence\n");
    await put(root, "node_modules/dev-tool/LICENSE", "dev only licence\n");
    await put(root, "node_modules/no-license-file/index.js", "");
  }
  await put(root, "src/app/page.tsx", "");
  await put(root, "src/app/login/page.tsx", "");
  await put(root, "src/app/(dashboard)/teams/page.tsx", "");
  await put(root, "src/app/(dashboard)/teams/[teamId]/page.tsx", "");
  await put(root, "src/app/@modal/page.tsx", "");
  await put(root, "src/app/_private/page.tsx", "");
  await put(root, "src/app/(.)photo/page.tsx", "");
  await put(root, "public/assets/icon.svg", "<svg/>");
  await put(root, "public/next.svg", "<svg/>");
}

export const EXPORT_FILES = {
  "index.html": "<html>home</html>",
  "login/index.html": "<html>login</html>",
  "teams/index.html": "<html>teams</html>",
  "index.txt": "rsc",
  "__next.__PAGE__.txt": "segment",
  "assets/icon.svg": "<svg/>",
  "next.svg": "<svg/>",
  "_next/static/chunks/app.js": "/*! chunk-lib v2 | MIT */\nconsole.log(1);\n/** @license Apache-2.0 */\nvar a=1;/* plain comment */\n",
  "_next/static/css/app.css": "/*! css-lib | MIT */\nbody{margin:0}\n/*! chunk-lib v2 | MIT */\n",
};

export async function writeExport(root, files = EXPORT_FILES) {
  for (const [relative, content] of Object.entries(files)) await put(root, relative, content);
}
