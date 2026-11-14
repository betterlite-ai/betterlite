import fs from "node:fs/promises";
import path from "node:path";

import { sha256Hex } from "../patches.mjs";
import { checkPin, loadPin, prepare, resolveSettings } from "../source.mjs";
import { SourceError } from "./errors.mjs";
import { installCancellation } from "./git.mjs";

async function snapshot(root) {
  const entries = [];
  const walk = async (directory, relative) => {
    const names = await fs.readdir(directory, { withFileTypes: true });
    names.sort((first, second) => (first.name < second.name ? -1 : first.name > second.name ? 1 : 0));
    for (const name of names) {
      const absolute = path.join(directory, name.name);
      const child = relative === "" ? name.name : `${relative}/${name.name}`;
      const stat = await fs.lstat(absolute);
      if (stat.isDirectory()) {
        entries.push({ path: child, mode: stat.mode & 0o777, sha256: null });
        await walk(absolute, child);
      } else if (stat.isSymbolicLink()) {
        entries.push({ path: child, mode: stat.mode & 0o777, sha256: sha256Hex(await fs.readlink(absolute)) });
      } else if (stat.isFile()) {
        entries.push({ path: child, mode: stat.mode & 0o777, sha256: sha256Hex(await fs.readFile(absolute)) });
      } else {
        throw new SourceError("unexpected-entry", `${absolute} is not a file, directory or symlink`);
      }
    }
  };
  await walk(root, "");
  return entries;
}

export async function compareTrees(first, second) {
  const left = await snapshot(first);
  const right = await snapshot(second);
  for (let index = 0; index < Math.max(left.length, right.length); index += 1) {
    const one = left[index];
    const two = right[index];
    if (one === undefined || two === undefined || one.path !== two.path || one.mode !== two.mode || one.sha256 !== two.sha256) {
      throw new SourceError("reproducibility", `the two preparations differ at ${one?.path ?? two?.path}: ${JSON.stringify(one)} and ${JSON.stringify(two)}`);
    }
  }
  return left;
}

export async function verifyRepeat(options = {}) {
  const settings = resolveSettings(options);
  const pin = options.pin ?? await loadPin(settings.pinPath);
  checkPin(pin);
  const verifyDir = settings.verifyDir ?? path.join(path.dirname(settings.cacheDir), "verify");
  await fs.mkdir(verifyDir, { recursive: true });
  const first = await fs.mkdtemp(path.join(verifyDir, "first-"));
  const second = await fs.mkdtemp(path.join(verifyDir, "second-"));
  try {
    const one = await prepare({ ...options, pin, outDir: first });
    await prepare({ ...options, pin, outDir: second });
    const compared = await compareTrees(first, second);
    return {
      commit: pin.commit,
      archiveSha256: one.archiveSha256,
      entries: compared.length,
      comparedPaths: compared.map((entry) => entry.path),
      patches: one.patches,
    };
  } finally {
    await fs.rm(first, { recursive: true, force: true }).catch(() => undefined);
    await fs.rm(second, { recursive: true, force: true }).catch(() => undefined);
  }
}

function optionValue(argv, index, name) {
  const value = argv[index + 1];
  if (value === undefined || value.startsWith("--")) {
    throw new SourceError("usage", `${name} needs a value`);
  }
  return value;
}

const VALUE_OPTIONS = {
  "--source-repo": "sourceRepo",
  "--cache": "cacheDir",
  "--out": "outDir",
  "--patches": "patchesDir",
  "--pin": "pinPath",
};

function parseArguments(argv) {
  const command = argv[0];
  const options = { repeat: false };
  for (let index = 1; index < argv.length; index += 1) {
    const argument = argv[index];
    if (Object.hasOwn(VALUE_OPTIONS, argument)) {
      options[VALUE_OPTIONS[argument]] = optionValue(argv, index, argument);
      index += 1;
    } else if (argument === "--timeout-ms") {
      const value = Number(optionValue(argv, index, argument));
      if (!Number.isSafeInteger(value) || value <= 0) throw new SourceError("usage", "--timeout-ms needs a positive integer");
      options.timeoutMs = value;
      index += 1;
    } else if (argument === "--repeat") {
      options.repeat = true;
    } else {
      throw new SourceError("usage", `unknown option ${argument}`);
    }
  }
  if (command !== "prepare" && command !== "verify") {
    throw new SourceError("usage", "usage: source.mjs <prepare|verify --repeat> [--source-repo <checkout>] [--pin <file>]");
  }
  if (command === "verify" && !options.repeat) {
    throw new SourceError("usage", "verify needs --repeat");
  }
  return { command, options };
}

export async function main() {
  installCancellation();
  const { command, options } = parseArguments(process.argv.slice(2));
  if (command === "prepare") {
    const result = await prepare(options);
    console.log(`prepared ${result.outDir} at ${result.commit} (${result.entries} entries, ${result.patches.length} patches)`);
    return;
  }
  const result = await verifyRepeat(options);
  console.log(`verified two preparations of ${result.commit} match (${result.entries} entries, ${result.patches.length} patches)`);
}
