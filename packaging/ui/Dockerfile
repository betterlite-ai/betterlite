ARG UI_BUILD_IMAGE=node:24.19-alpine3.24@sha256:d32cdf619f63fe0471182d08996dd516c6275bb5fd31ae06e55a570bd9e1ad43
# The builder LiteLLM's own Dockerfile pins for its admin UI; it matches ui/litellm-dashboard/.nvmrc

FROM --platform=$BUILDPLATFORM $UI_BUILD_IMAGE AS dependencies
ENV NEXT_TELEMETRY_DISABLED=1 \
    npm_config_fund=false \
    npm_config_audit=false
WORKDIR /work/dashboard
COPY ui/litellm-dashboard/package.json ui/litellm-dashboard/package-lock.json ui/litellm-dashboard/.npmrc ./
RUN --mount=type=cache,target=/root/.npm npm ci --prefer-offline

FROM dependencies AS application
COPY ui/litellm-dashboard/ ./

# Dependencies are in place, so nothing below reaches the network
FROM application AS compiled
ARG BUILD_ID
ARG API_BASE=""
RUN --network=none <<'SCRIPT'
set -eu
unset NEXT_PUBLIC_BASE_URL NEXT_PUBLIC_USE_REWRITES
if [ -n "$API_BASE" ]; then
  export NEXT_PUBLIC_BASE_URL="$API_BASE"
fi
NODE_ENV=production BETTERLITE_BUILD_ID="${BUILD_ID:?}" npm run build
SCRIPT

FROM compiled AS bundled
ARG MANIFEST_INPUTS
COPY --from=tools bundle.mjs /opt/bundle.mjs
COPY LICENSE /opt/LiteLLM-LICENSE.txt
RUN --network=none node /opt/bundle.mjs finalize \
    --exported /work/dashboard/out --app /work/dashboard --license /opt/LiteLLM-LICENSE.txt --bundle /bundle

FROM scratch AS export
COPY --from=bundled /bundle /bundle

FROM application AS checks
COPY --from=tools bundle.mjs /opt/bundle.mjs
COPY --from=checkplan plan.json /opt/plan.json
COPY --from=checkplan tests/ tests/betterlite/
RUN --network=none node /opt/bundle.mjs check --plan /opt/plan.json --app /work/dashboard

FROM scratch AS checked
COPY --from=checks /opt/plan.json /plan.json
