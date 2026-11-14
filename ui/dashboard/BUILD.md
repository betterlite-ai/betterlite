# Dashboard bundle build

`tools/dashboard/build.sh` turns the prepared dashboard source into the static sidecar that sits next to the betterlite binary. The build runs in the Node image pinned by digest in `packaging/ui/Dockerfile`, so the same inputs give the same files on any machine with Docker and buildx.

## Commands

    bash tools/dashboard/build.sh key
    bash tools/dashboard/build.sh build [--api-base <url>] [--rebuild]
    bash tools/dashboard/build.sh check <tests-directory>
    node tools/dashboard/install.mjs --bundle target/dashboard/dist --bin <path to betterlite>
    bash tools/dashboard/check-build.sh

`key` prints the cache key and needs neither Docker nor the network. `build` writes `target/dashboard/dist` (`ui/` and `manifest.json`) and skips the work when the manifest in `dist` already carries the current key. `--rebuild` ignores the manifest. `--source-repo <checkout>` and `--out <directory>` go to the source intake and the work directory.

`check-build.sh` runs two clean builds with `--rebuild`, compares their path, mode and content digests and a deterministic tar, checks the notices and every output kind, installs the bundle beside a temporary executable and proves stale chunks disappear. It needs Docker with buildx; when the CLI is missing, as in the gate's task container, which carries the host's socket but no client, it installs `docker-cli` and `docker-buildx` from the distribution first.

`--api-base` builds the dashboard against another API origin. Without it the export is same-origin: `NEXT_PUBLIC_BASE_URL` and `NEXT_PUBLIC_USE_REWRITES` are removed from the build environment, whatever the caller has set. The value must be an `http` or `https` URL without credentials, and it is part of the cache key and the build id. `UI_BASE_PATH` and `--ui-base-path` are accepted and ignored, as LiteLLM's `next.config.mjs` never reads them.

## What the build does

1. `tools/dashboard/source.mjs prepare` publishes the verified source with the patches in `ui/dashboard/patches` applied.
2. In the `dependencies` stage the image copies `package.json`, `package-lock.json` and `.npmrc` alone and runs `npm ci --prefer-offline` at `/work/dashboard`, so the dependency layer changes only with the lock.
3. The `compiled` stage runs `npm run build` with `--network=none`, `NODE_ENV=production`, telemetry, fund and audit switched off and the build id from `10-build.patch`.
4. The `bundled` stage runs `bundle.mjs finalize`, again without network. It checks the layout, writes the notices and the `.litellm_ui_ready` marker, and records `manifest.json`.
5. The `export` stage is `FROM scratch` and holds `/bundle` only, so the output is the bundle and nothing else from the image.

The build runs on `$BUILDPLATFORM`: the files do not depend on the target CPU, so one build serves every binary.

`bundle.mjs` is only the command line entrypoint. Its jobs live in `tools/dashboard/bundle/`: `inputs.mjs` holds the cache key, the build id and the BuildKit cache flags, `checks.mjs` the check plan and its runner, `finalize.mjs` the layout check, the notices and the manifest. `patches.mjs` owns patch discovery, record validation and application for both the source intake and the bundle.

## Bundle contents

`ui/` holds `index.html`, `login/index.html`, every static route of the application, `_next`, the RSC `.txt` payloads with their `__next` segment files, the `public` assets and the `.litellm_ui_ready` marker. The finalize step fails when a page, a payload or a public asset is missing, when the export holds a symlink, or when it holds `logo.png`, `logo_dark.png`, `logo_monogram.png`, `logo_monogram_dark.png` or `cached_logo.jpg`. File modes are normalised to 0644 and directories to 0755.

`ui/notices/LiteLLM-LICENSE.txt` is the upstream `LICENSE` byte for byte. `ui/notices/npm-dependencies.txt` lists the licence files of the non-dev packages in the lock file and the `/*!`, `@license` and `@preserve` comments from the emitted scripts and styles.

`manifest.json` starts with the cache key and lists the build id, the API base, the builder image, the source commit and archive hash, the patch names, the Node and npm versions, the file count and a digest of every path, mode and content hash.

## Cache key

The key is the SHA-256 of the pinned source archive, each patch and its record, the lock file and `.npmrc` hashes from `source.pin.json`, the builder image digest, `packaging/ui/Dockerfile`, `build.sh`, every build tool module (`bundle.mjs`, `patches.mjs` and each file under `bundle/`) and the API base, so editing any module the image runs builds a new bundle. The build id Next.js stamps into the export comes from the archive hash, the patches and the API base, so it does not change when only a build script changes.

## BuildKit cache in CI

Set `DASHBOARD_BUILDKIT_CACHE=read` to read the GitHub Actions cache and `read-write` to read and write it. Both need `ACTIONS_RESULTS_URL` and `ACTIONS_RUNTIME_TOKEN` in the environment, which a workflow step exports. When either is missing, or the mode is unset, the build uses the local BuildKit cache only. Any other mode stops the build. The token is read by buildx from its own environment: it is never passed as a build argument, never written to the manifest and never printed.

## Shared checks

`build.sh check <tests-directory>` runs inside the same image on the prepared source. The directory is copied to `tests/betterlite` in the application and run with Vitest, then the patched TypeScript files and those tests go through the upstream ESLint and Prettier configuration. An optional `upstream-tests.json` in the directory lists more upstream test files to run, as a JSON array of paths relative to the application root, for example `["src/lib/http/resolveApiBase.test.ts"]`. A path outside the application, a file that is not a `.test.ts` or `.test.tsx`, or one that does not exist fails the plan.

## Install

`install.mjs` copies `ui/` from a finished bundle to a `ui` directory beside the binary, then swaps it in. The old directory goes aside first and is removed after the new one is in place, so files from an earlier build, such as content-hashed chunks and removed routes, never stay behind. It refuses a bundle without the ready marker or `manifest.json`, and a bundle that holds a symlink.
