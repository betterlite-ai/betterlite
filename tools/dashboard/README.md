# Dashboard source intake

The dashboard is LiteLLM's Next.js application, kept as a verified build input instead of committed source. This tool fetches one pinned commit, verifies its archive and lockfile, extracts the MIT paths, applies the reviewed patches and publishes `target/dashboard/source`. B9091 builds that tree into the distributed sidecar; B9092 adds its branding through a patch here.

`source.mjs` is the entry point and keeps the pin, protected-file and publication code. The modules under `source/` hold one job each: `git.mjs` runs Git with isolated configuration, timeouts and process groups, `tar.mjs` reads the verified archive and extracts the selected paths, `cache.mjs` owns the commit cache and archive verification, and `cli.mjs` holds the reproducibility comparison and the command line.

## Commands

    node tools/dashboard/source.mjs prepare
    node tools/dashboard/source.mjs verify --repeat

`prepare` publishes `target/dashboard/source` with the upstream repository layout. `verify --repeat` prepares the same pinned source twice in separate clean directories and compares every relative path, file mode and file hash, so a nondeterministic archive, extraction or patch fails the check.

Options:

- `--source-repo <checkout>` uses an existing read-only checkout that contains the pinned commit. Its commit object, dashboard tree and archive pass exactly the same verification as a fetched one, and its working tree is never read. `--source-repo ../litellm` prepares without network access when that checkout sits beside the task worktree.
- `--pin <file>` reads another pin file, `--cache <dir>` and `--out <dir>` move the cache and the published tree, and `--patches <dir>` reads patches from another folder.
- `--timeout-ms <ms>` bounds each Git call; the defaults are 60 seconds for most local calls, 120 seconds for the archive and cache verification, and 10 minutes for the fetch.

## Pin and cache

`source.pin.json` holds the repository, the full commit id, the exported paths in order, the archive SHA-256, the `ui/litellm-dashboard` tree id and the SHA-256 of every protected upstream file.

The private cache lives in `target/dashboard/cache`. `repo.git` is a bare repository that receives only the pinned commit, fetched with `--no-tags --depth=1`, and `archive.tar` is the verified export. A cache hit is never evidence: every run hashes `archive.tar` against the pin before extracting, and a mismatch stops the run. A cache repository that fails `git fsck` stops it too. Deleting `target/dashboard/cache` makes the next run fetch again.

Git runs with a fresh isolated configuration: `GIT_CONFIG_GLOBAL` and `GIT_CONFIG_SYSTEM` point at `/dev/null`, `GIT_CONFIG_NOSYSTEM` and `GIT_ATTR_NOSYSTEM` are set, inherited `GIT_DIR`, `GIT_WORK_TREE`, `GIT_OBJECT_DIRECTORY` and `GIT_CONFIG_COUNT`-style variables are removed, and the archive uses `-c tar.umask=0002` with `-c core.attributesFile=/dev/null`. The archive always names the commit object, never a tag, a tree or the working tree, so its bytes and the commit time come from the pin alone. `git apply` runs with `GIT_CEILING_DIRECTORIES` set to the parent of the prepared tree, so an enclosing betterlite repository cannot redirect patch paths.

## What enters the tree

The export is `LICENSE`, `ui/**` and the five `litellm/proxy` logo files named in the pin. Extraction refuses an absolute path, a `..` segment, a path outside that export and a symlink that resolves outside the destination. It drops any path with a component named `enterprise` or ending in `_enterprise`, and the `_experimental/out` bundle listed in the pin. The full upstream LICENSE, including the Berri AI notice and permission text, stays in the tree.

`LICENSE`, `package.json`, `package-lock.json` and `.npmrc` are hashed before and after the patches. Any change, including a patch that tries to touch them, fails the run: the dependency files stay byte-identical to the commit.

## Patch contract

Patches are checked into `ui/dashboard/patches` and applied in filename order. Each `<name>.patch` needs an adjacent `<name>.json`:

    {
      "targets": [
        {
          "path": "ui/litellm-dashboard/next.config.mjs",
          "preimage": "<sha256 hex>",
          "postimage": "<sha256 hex>"
        }
      ]
    }

`preimage` is `null` for a file the patch creates and `postimage` is `null` for one it deletes. The record must list exactly the paths `git apply --numstat` reports for the patch, every path must be inside the selected export, and no path may name a protected file. Preimages are checked against the current tree before the patch runs, postimages after it, so a later patch sees the output of an earlier one. Rename and copy patches are not part of the contract.

Each patch runs `git apply --check --binary` before `git apply --binary`. A failed hunk, a wrong hash, an unlisted path or a missing record stops the run. Every preparation starts from the pristine verified export and applies the patches again; an old prepared tree is never patched in place.

## Failures

A run that fails before publication removes its staging directory and leaves `target/dashboard/source` exactly as it was. Only a fully verified, fully patched tree is renamed into place. Timeouts kill the whole Git process group, and `SIGINT` or `SIGTERM` does the same before the process exits.
