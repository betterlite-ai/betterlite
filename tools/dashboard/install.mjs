import fs from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { parseArgs } from "node:util";

import { listTree } from "./bundle/inputs.mjs";
import { PatchError } from "./patches.mjs";

async function exists(file) {
  return fs.stat(file).then(() => true, () => false);
}

export async function installBundle({ bundle, bin }) {
  const source = path.join(path.resolve(bundle), "ui");
  if (!(await exists(path.join(source, ".litellm_ui_ready"))) || !(await exists(path.join(path.resolve(bundle), "manifest.json")))) {
    throw new PatchError("bundle", `${bundle} is not a finished dashboard bundle`);
  }
  const binary = path.resolve(bin);
  if (!(await fs.stat(binary).catch(() => null))?.isFile()) throw new PatchError("bin", `${bin} is not a file`);
  await listTree(source);
  const target = path.join(path.dirname(binary), "ui");
  const staging = `${target}.new-${process.pid}`;
  const trash = `${target}.old-${process.pid}`;
  await fs.rm(staging, { recursive: true, force: true });
  await fs.cp(source, staging, { recursive: true });
  const hadOld = await fs.rename(target, trash).then(() => true, (error) => {
    if (error.code === "ENOENT") return false;
    throw error;
  });
  try {
    await fs.rename(staging, target);
  } catch (error) {
    if (hadOld) await fs.rename(trash, target);
    throw error;
  }
  if (hadOld) await fs.rm(trash, { recursive: true, force: true });
  return target;
}

if (process.argv[1] !== undefined && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const { values } = parseArgs({ options: { bundle: { type: "string" }, bin: { type: "string" } } });
  if (values.bundle === undefined || values.bin === undefined) {
    console.error("usage: install.mjs --bundle <dist> --bin <betterlite>");
    process.exit(2);
  }
  installBundle({ bundle: values.bundle, bin: values.bin })
    .then((target) => console.log(`installed ${target}`))
    .catch((error) => {
      console.error(`error: ${error.message}`);
      process.exit(1);
    });
}
