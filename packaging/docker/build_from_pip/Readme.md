# Build the betterlite proxy image from the published package

`Dockerfile.build_from_pip` installs a released betterlite distribution from a Python package index and starts its proxy. It builds the artifact users install, so the image checks the published release rather than the code in this checkout.

## When to use this recipe

Use it when security or compliance rules require images built from released, versioned artifacts instead of a source checkout. Every Python package in the image carries a published version, both base images are pinned by digest, and the Prisma engines are baked in at build time so the container downloads no binary at startup.

Use `packaging/docker/Dockerfile` for the native source image. That one compiles the workspace and ships the `betterlite` and `betterlite-db-admin` binaries. This recipe answers the other question: does the package published on the index start and serve the configuration operators run.

## Build

Run the build from the repository root. The context has to include the entrypoint in `packaging/docker/` and the pinned schema in `crates/betterlite-db/src/schema_contract/fixtures/root/`.

```sh
docker build \
  -f packaging/docker/build_from_pip/Dockerfile.build_from_pip \
  -t betterlite:published \
  .
```

`BETTERLITE_VERSION` names a published release and defaults to the latest one at the time of writing. It never follows the version in this checkout, so pass `--build-arg BETTERLITE_VERSION=<version>` to test another release. The release must publish the `proxy` and `proxy-runtime` extras; a version without them fails at the install step.

## Run against the mock provider

`litellm_config.yaml` defines one model, `gpt-4`, on the fake OpenAI provider, and asks the proxy to send alerts to Slack. Mount it at `/app/config.yaml` and point `FAKE_OPENAI_API_BASE` at a mock OpenAI server, since the configuration reads that variable and has no default.

```sh
docker run --rm -p 4000:4000 \
  --add-host host.docker.internal:host-gateway \
  -e LITELLM_MASTER_KEY=sk-1234 \
  -e FAKE_OPENAI_API_BASE=http://host.docker.internal:8190 \
  -v "$(pwd)/packaging/docker/build_from_pip/litellm_config.yaml:/app/config.yaml" \
  betterlite:published \
  --config /app/config.yaml --port 4000
```

The image starts `/app/docker/prod_entrypoint.sh`, the same entrypoint as the source image, and its default command is `--port 4000`. Prisma generation runs at build time only. The baked CLI and engines serve Python callbacks, and no migration or code generation runs when the container starts.
