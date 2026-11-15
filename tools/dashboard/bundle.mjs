import { execFileSync } from "node:child_process";
import fs from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { parseArgs } from "node:util";

import { runChecks, writePlan } from "./bundle/checks.mjs";
import { finalize } from "./bundle/finalize.mjs";
import { bundleInputs, cacheFlags } from "./bundle/inputs.mjs";
import { PatchError } from "./patches.mjs";

export async function main(argv, env) {
  const { positionals, values } = parseArgs({
    args: argv,
    allowPositionals: true,
    options: {
      "api-base": { type: "string", default: "" },
      "repo-root": { type: "string", default: path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..") },
      tests: { type: "string" },
      app: { type: "string", default: "/work/dashboard" },
      out: { type: "string" },
      exported: { type: "string" },
      license: { type: "string" },
      bundle: { type: "string" },
      plan: { type: "string" },
    },
  });
  const repoRoot = path.resolve(values["repo-root"]);
  const appRoot = path.resolve(values.app);
  switch (positionals[0]) {
    case "key":
    case "build-id":
    case "inputs": {
      const inputs = await bundleInputs({ repoRoot, apiBase: values["api-base"] });
      const printed = { key: inputs.key, "build-id": inputs.buildId, inputs: JSON.stringify(inputs) };
      process.stdout.write(`${printed[positionals[0]]}\n`);
      break;
    }
    case "cache-flags":
      process.stdout.write(`${cacheFlags(env).join(" ")}\n`);
      break;
    case "plan":
      await writePlan({ testsDir: path.resolve(values.tests), appRoot, repoRoot, out: path.resolve(values.out) });
      break;
    case "finalize": {
      const npm = execFileSync("npm", ["--version"], { encoding: "utf8" }).trim();
      const manifest = await finalize({
        exported: path.resolve(values.exported),
        appRoot,
        license: path.resolve(values.license),
        bundle: path.resolve(values.bundle),
        inputs: JSON.parse(env.MANIFEST_INPUTS ?? "{}"),
        versions: { node: process.version, npm },
      });
      process.stdout.write(`bundle ${manifest.key}: ${manifest.files} files, node ${manifest.node}, npm ${manifest.npm}\n`);
      break;
    }
    case "check": {
      const planFile = path.resolve(values.plan);
      runChecks({ appRoot, plan: JSON.parse(await fs.readFile(planFile, "utf8")) });
      break;
    }
    default:
      throw new PatchError("usage", "usage: bundle.mjs key|build-id|inputs|cache-flags|plan|finalize|check [options]");
  }
}

if (process.argv[1] !== undefined && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main(process.argv.slice(2), process.env).catch((error) => {
    console.error(`error: ${error.message}`);
    process.exit(1);
  });
}
