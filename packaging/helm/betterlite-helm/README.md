# betterlite Helm chart

betterlite is a LiteLLM-compatible AI gateway written in Rust. This chart installs the gateway as a Deployment with a Service, and can add a migrations Job that applies the database schema, a bundled PostgreSQL database and a bundled Redis, so a new cluster needs no external services.

## Prerequisites

- Kubernetes 1.21 or newer.
- Helm 3.
- A persistent volume provisioner when `db.deployStandalone` is true, because the bundled PostgreSQL keeps its data on a volume.
- Network access from the machine running Helm to `oci://registry-1.docker.io/bitnamicharts`, where the PostgreSQL and Redis subcharts come from.

`db.useStackgresOperator` is accepted but not implemented; the chart deploys no Stackgres cluster.

## Quick start

Install with the bundled PostgreSQL and a master key you choose:

```bash
helm install betterlite packaging/helm/betterlite-helm \
  --namespace betterlite --create-namespace \
  --set masterkey=sk-your-master-key \
  --set db.deployStandalone=true
```

`db.deployStandalone` defaults to `true`, so this release creates a PostgreSQL instance and a migrations Job alongside the gateway. If both `masterkey` and `masterkeySecretName` are left unset, the chart generates a key on the first install and stores it in the release's master key Secret, `betterlite-masterkey` for this release name; later upgrades reuse it:

```bash
kubectl -n betterlite get secret betterlite-masterkey -o jsonpath='{.data.masterkey}' | base64 -d
```

To use an existing database and a master key from your own Secret:

```bash
kubectl create namespace betterlite
kubectl create secret generic betterlite-masterkey -n betterlite \
  --from-literal=masterkey=sk-your-master-key
kubectl create secret generic postgres -n betterlite \
  --from-literal=username=betterlite \
  --from-literal=password=change-me

helm install betterlite packaging/helm/betterlite-helm \
  --namespace betterlite \
  --set masterkeySecretName=betterlite-masterkey \
  --set db.deployStandalone=false \
  --set db.useExisting=true \
  --set db.endpoint=postgres.example.com \
  --set db.database=betterlite
```

## Values

Defaults come from `values.yaml`. Every key can be set with `--set` or in a values file passed with `-f`. Nested keys use dots on the command line, for example `--set db.connectionPool.enabled=true`.

### Deployment

| Key | Default | Meaning |
| --- | --- | --- |
| `replicaCount` | `1` | Gateway pods. The chart leaves `replicas` out of the Deployment when `autoscaling.enabled` or `keda.enabled` is on. |
| `numWorkers` | unset | Worker processes per pod, passed as `--num_workers`. Commented out in `values.yaml`; unset leaves the server default. |
| `image.repository` | `ghcr.io/betterlite-ai/betterlite` | Gateway image. The same image serves the API, the Admin UI and the migrations command. |
| `image.pullPolicy` | `Always` | Pull policy for the image. |
| `image.tag` | `""` | Image tag. Empty uses the chart `appVersion`. |
| `imagePullSecrets` | `[]` | Secrets for pulling the gateway and migration images. |
| `nameOverride` | `betterlite` | Name used in resource names instead of the chart name. |
| `fullnameOverride` | `""` | Replaces the full release name in resource names. |
| `serviceAccount.create` | `false` | Create a ServiceAccount for the pods. |
| `serviceAccount.automount` | `true` | Mount the ServiceAccount token. |
| `serviceAccount.annotations` | `{}` | Annotations on the ServiceAccount. |
| `serviceAccount.name` | `""` | ServiceAccount name. Empty uses the generated name, or `default` when none is created. |
| `deploymentAnnotations` | `{}` | Annotations on the Deployment. |
| `deploymentLabels` | `{}` | Extra labels on the Deployment. |
| `deploymentMinReadySeconds` | `0` | Seconds a pod must be ready before it counts as available. |
| `podAnnotations` | `{}` | Annotations on the gateway pods. Values are rendered as templates. |
| `podLabels` | `{}` | Extra labels on the gateway pods. |
| `strategy` | `{}` | Deployment strategy, for example a RollingUpdate with `maxUnavailable` 0. |
| `terminationGracePeriodSeconds` | `90` | Shutdown grace period for the gateway pods. |
| `topologySpreadConstraints` | `[]` | Spread constraints for the gateway pods. |
| `podSecurityContext` | `{}` | Security context for the pods. |
| `securityContext` | `{}` | Security context for the gateway, collector and migrations Job containers. |
| `environmentSecrets` | `[]` | Secret names whose keys become environment variables, so a `proxy_config` can reach them as `os.environ/<name>`. |
| `environmentConfigMaps` | `[]` | ConfigMap names whose keys become environment variables. |
| `service.type` | `ClusterIP` | Service type. |
| `service.port` | `4000` | Port of the Service and of the gateway inside the pod. The Admin UI answers on the same port. |
| `service.annotations` | `{}` | Annotations on the Service. |
| `service.loadBalancerClass` | `""` | LoadBalancer class, used only when `service.type` is `LoadBalancer`. |
| `listen` | `0.0.0.0` | Address the gateway binds inside the container. |
| `livenessProbe.path` | `/health/liveliness` | Liveness probe path. |
| `livenessProbe.initialDelaySeconds` | `0` | Liveness probe delay. |
| `livenessProbe.periodSeconds` | `15` | Liveness probe period. |
| `livenessProbe.timeoutSeconds` | `5` | Liveness probe timeout. |
| `livenessProbe.successThreshold` | `1` | Liveness successes needed to be considered up. |
| `livenessProbe.failureThreshold` | `5` | Liveness failures that restart the container. |
| `readinessProbe.path` | `/health/readiness` | Readiness probe path. |
| `readinessProbe.initialDelaySeconds` | `0` | Readiness probe delay. |
| `readinessProbe.periodSeconds` | `10` | Readiness probe period. |
| `readinessProbe.timeoutSeconds` | `5` | Readiness probe timeout. |
| `readinessProbe.successThreshold` | `1` | Readiness successes needed to take traffic. |
| `readinessProbe.failureThreshold` | `3` | Readiness failures that remove the pod from the Service. |
| `startupProbe.path` | `/health/readiness` | Startup probe path. |
| `startupProbe.initialDelaySeconds` | `0` | Startup probe delay. |
| `startupProbe.periodSeconds` | `10` | Startup probe period. |
| `startupProbe.timeoutSeconds` | `5` | Startup probe timeout. |
| `startupProbe.successThreshold` | `1` | Startup successes needed to hand over to the liveness probe. |
| `startupProbe.failureThreshold` | `30` | Startup failures that restart the container. |
| `resources` | `{}` | CPU and memory for the gateway container. Unset keeps the chart installable on small clusters; set requests and limits in production. |
| `lifecycle` | `{}` | Lifecycle hooks for the gateway container. |
| `volumes` | `[]` | Extra volumes on the Deployment and the migrations Job. |
| `volumeMounts` | `[]` | Extra volume mounts on the gateway, collector and migrations Job containers. |
| `nodeSelector` | `{}` | Node selector for the pods. |
| `tolerations` | `[]` | Tolerations for the pods. |
| `affinity` | `{}` | Affinity rules for the pods. |
| `logLevel` | `INFO` | Sets `LITELLM_LOG`. Empty stops the injection, and defining `LITELLM_LOG` in `envVars` turns it off too. |
| `envVars` | `{}` | Extra environment variables as a map of key and value. |
| `extraEnvVars` | `{}` | Extra environment variables as a list of Kubernetes env entries. |
| `command` | `{}` | Overrides the gateway container command. |
| `args` | `{}` | Overrides the gateway container args. Empty runs the gateway with `--config /etc/betterlite/config.yaml`. |
| `extraContainers` | `[]` | Sidecar containers, rendered as templates. |
| `extraInitContainers` | `[]` | Init containers, rendered as templates. |
| `extraResources` | `[]` | Extra Kubernetes resources deployed with the release. |
| `test.kubectlImage` | `docker.io/bitnamilegacy/kubectl:1.29.2-debian-12-r3` | Image for the `helm test` pods, so a cluster can point them at a mirror. |

### Master key and proxy config

| Key | Default | Meaning |
| --- | --- | --- |
| `masterkey` | `""` | Explicit master key. Unset reuses the stored key or generates `sk-` plus 18 characters. |
| `masterkeySecretName` | `""` | Existing Secret that holds the master key. The chart then creates no master key Secret. |
| `masterkeySecretKey` | `""` | Key inside that Secret. Empty uses `masterkey`. |
| `proxyConfigMap.create` | `true` | Render `proxy_config` into a ConfigMap and mount it. |
| `proxyConfigMap.name` | unset | Existing ConfigMap to mount when `create` is false. |
| `proxyConfigMap.key` | `config.yaml` | Key inside the ConfigMap that holds the config file. |
| `proxy_config` | Two example models and `general_settings.master_key: os.environ/PROXY_MASTER_KEY` | The proxy config file, rendered as `config.yaml`. At least one model is required for the gateway to start. |

### Metrics and collector

| Key | Default | Meaning |
| --- | --- | --- |
| `metricsServer.enabled` | `false` | Serve Prometheus metrics from a separate process, adding a `metrics` port and a metrics Service named after the release. |
| `metricsServer.port` | `4001` | Metrics port. It must differ from `service.port`, and it has no virtual-key auth, so keep it off public ingress. |
| `collector.enabled` | `false` | Run the spend collector as a sidecar, so proxy workers only serialize an event and go back to serving inference. |
| `collector.address` | `unix:///var/run/litellm/collector.sock` | Collector address. A `unix://` address creates a shared emptyDir for the socket. |
| `collector.bufferSize` | `1000` | Events each worker holds in memory while the sidecar is slow or restarting. |
| `collector.onUnavailable` | `fallback` | `fallback` runs the pipeline in the worker when the sidecar is unreachable or the buffer is full; `drop` counts and discards the event instead. |
| `collector.drainTimeoutSeconds` | `10` | How long workers keep pushing buffered events on shutdown, and how long the sidecar serves open connections. |
| `collector.command` | `["betterlite", "collector"]` | Collector container command. |
| `collector.resources` | `{}` | CPU and memory for the collector container, sized independently of the proxy. |
| `collector.scaleOnProxyContainerCpu` | `false` | With `autoscaling.enabled`, scale on a ContainerResource metric of the proxy container only, so the sidecar's CPU never scales inference replicas. Needs Kubernetes 1.30 or newer. |

### Autoscaling and KEDA

| Key | Default | Meaning |
| --- | --- | --- |
| `autoscaling.enabled` | `false` | Render an autoscaling/v2 HorizontalPodAutoscaler for the Deployment. |
| `autoscaling.minReplicas` | `1` | Lowest replica count. |
| `autoscaling.maxReplicas` | `100` | Highest replica count. |
| `autoscaling.targetCPUUtilizationPercentage` | `60` | CPU utilization target, the documented recommendation. |
| `autoscaling.targetMemoryUtilizationPercentage` | unset | Memory utilization target. Left unset because the query engine's memory ratchets to the pod's worst write, so a memory target scales up and never back in. |
| `autoscaling.behavior` | unset | HPA behavior block. |
| `autoscaling.targetRequestsPerSecond` | `""` | Per-pod requests-per-second target, rendered as a Pods metric named `litellm_requests_per_second`; needs a Prometheus Adapter. |
| `autoscaling.targetTokensPerSecond` | `""` | Per-pod tokens-per-second target, rendered as a Pods metric named `litellm_tokens_per_second`; needs a Prometheus Adapter. |
| `keda.enabled` | `false` | Render a KEDA ScaledObject. KEDA scales the Deployment only while `autoscaling.enabled` is false. |
| `keda.minReplicas` | `1` | Lowest replica count. |
| `keda.maxReplicas` | `100` | Highest replica count. |
| `keda.pollingInterval` | `30` | Seconds between KEDA metric polls. |
| `keda.cooldownPeriod` | `300` | Seconds KEDA waits before scaling back to the minimum. |
| `keda.fallback` | unset | KEDA fallback block. `failureThreshold` defaults to 3 and `replicas` to `keda.maxReplicas`. |
| `keda.restoreToOriginalReplicaCount` | `false` | Let KEDA restore the original replica count when the ScaledObject is removed. |
| `keda.scaledObject.annotations` | `{}` | Annotations on the ScaledObject. |
| `keda.triggers` | `[]` | KEDA triggers. |
| `keda.prometheus.serverAddress` | `""` | Prometheus address for the first-class triggers. Required when either target below is set. |
| `keda.prometheus.requestsPerSecond` | `""` | Per-replica requests-per-second target built on `litellm_proxy_total_requests_metric_total`. |
| `keda.prometheus.tokensPerSecond` | `""` | Per-replica tokens-per-second target built on `litellm_total_tokens_metric_total`. |
| `keda.behavior` | `{}` | HPA behavior for the ScaledObject. |

### Ingress, monitoring and disruption budget

| Key | Default | Meaning |
| --- | --- | --- |
| `ingress.enabled` | `false` | Render an Ingress. |
| `ingress.className` | `nginx` | Ingress class. |
| `ingress.labels` | `{}` | Extra labels on the Ingress. |
| `ingress.annotations` | `{}` | Annotations on the Ingress. |
| `ingress.hosts` | One host, `api.example.local`, with path `/` | Hosts and paths of the Ingress. |
| `ingress.tls` | `[]` | TLS entries for the Ingress. |
| `serviceMonitor.enabled` | `false` | Render a Prometheus Operator ServiceMonitor. It scrapes the `metrics` Service port when `metricsServer.enabled` is true, otherwise the `http` port, at `/metrics/`. |
| `serviceMonitor.labels` | `{}` | Extra labels on the ServiceMonitor. |
| `serviceMonitor.annotations` | `{}` | Annotations on the ServiceMonitor. |
| `serviceMonitor.interval` | `15s` | Scrape interval. |
| `serviceMonitor.scrapeTimeout` | `10s` | Scrape timeout. |
| `serviceMonitor.relabelings` | `[]` | Relabeling rules. |
| `serviceMonitor.namespaceSelector.matchNames` | `[]` | Namespaces to scrape. Empty means the release namespace. |
| `pdb.enabled` | `false` | Render a PodDisruptionBudget. |
| `pdb.minAvailable` | `null` | Minimum available pods during voluntary disruptions. Set exactly one of this and `maxUnavailable`; `minAvailable` wins when both are set. |
| `pdb.maxUnavailable` | `null` | Maximum unavailable pods during voluntary disruptions. Renders as 1 when neither field is set. |
| `pdb.annotations` | `{}` | Annotations on the budget. |
| `pdb.labels` | `{}` | Extra labels on the budget. |

### Database

| Key | Default | Meaning |
| --- | --- | --- |
| `db.useExisting` | `false` | Use an existing PostgreSQL server; credentials come from `db.secret`. |
| `db.endpoint` | `localhost` | PostgreSQL host when `useExisting` is true and `db.secret.endpointKey` is empty. |
| `db.database` | `litellm` | Database name. |
| `db.url` | `postgresql://$(DATABASE_USERNAME):$(DATABASE_PASSWORD)@$(DATABASE_HOST)/$(DATABASE_NAME)` | `DATABASE_URL` for the existing database. The client fills the placeholders from the variables above. |
| `db.secret.name` | `postgres` | Secret with the database credentials. |
| `db.secret.usernameKey` | `username` | Key holding the user name. |
| `db.secret.passwordKey` | `password` | Key holding the password. |
| `db.secret.endpointKey` | `""` | When set, `DATABASE_HOST` comes from this secret key instead of `db.endpoint`. |
| `db.secret.readReplicaUrlKey` | `""` | When set, `DATABASE_URL_READ_REPLICA` comes from this secret key instead of `db.readReplicaUrl`. |
| `db.secret.readReplicaEndpointKey` | `""` | When set, `DATABASE_READER_HOST` comes from this secret key, so `readReplicaUrl` can be composed from the individual parts. Ignored when `readReplicaUrlKey` is set. |
| `db.readReplicaUrl` | `""` | Reader URL for read-replica routing. Empty keeps single-database behavior. |
| `db.connectionPool.enabled` | `false` | Run PgBouncer in the pod in transaction mode, shared by every worker. Without it a pod's footprint is workers times `connection_limit` and grows with every replica. |
| `db.connectionPool.maxDbConnections` | `20` | Upstream database connections one pod holds at most. |
| `db.connectionPool.maxClientConn` | `1000` | Client connections the in-pod pool accepts. |
| `db.useStackgresOperator` | `false` | Accepted for compatibility with LiteLLM values; not implemented. |
| `db.deployStandalone` | `true` | Deploy the bundled PostgreSQL subchart and point the gateway and the migrations Job at it. |

### Bundled PostgreSQL

| Key | Default | Meaning |
| --- | --- | --- |
| `postgresql.architecture` | `standalone` | Subchart architecture. |
| `postgresql.image.repository` | `bitnamilegacy/postgresql` | PostgreSQL image repository. |
| `postgresql.image.tag` | `16.2.0-debian-12-r6` | PostgreSQL image tag. Pinned: the chart refuses to render an empty or `latest` tag, because starting another major version against an existing data directory leaves the database unreadable. |
| `postgresql.volumePermissions.image.repository` | `bitnamilegacy/os-shell` | Volume permissions image. |
| `postgresql.volumePermissions.image.tag` | `12-debian-12-r16` | Volume permissions image tag. |
| `postgresql.metrics.image.repository` | `bitnamilegacy/postgres-exporter` | Metrics exporter image. |
| `postgresql.metrics.image.tag` | `0.15.0-debian-12-r14` | Metrics exporter image tag. |
| `postgresql.auth.username` | `litellm` | Database user created by the subchart. |
| `postgresql.auth.database` | `litellm` | Database created by the subchart. |
| `postgresql.auth.password` | `NoTaGrEaTpAsSwOrD` | Password for that user. Change it. |
| `postgresql.auth.postgres-password` | `NoTaGrEaTpAsSwOrD` | Password for the `postgres` superuser. Change it. |

Other values under `postgresql` pass straight to the Bitnami subchart; see its documentation.

### Redis

| Key | Default | Meaning |
| --- | --- | --- |
| `redis.enabled` | `false` | Deploy the bundled Redis subchart and wire `REDIS_HOST`, `REDIS_PORT` and `REDIS_PASSWORD` into the gateway. |
| `redis.architecture` | `standalone` | Subchart architecture. |
| `redis.image.repository` | `bitnamilegacy/redis` | Redis image repository. |
| `redis.image.tag` | `7.2.4-debian-12-r9` | Redis image tag. |
| `redis.sentinel.image.repository` | `bitnamilegacy/redis-sentinel` | Sentinel image repository. |
| `redis.sentinel.image.tag` | `7.2.4-debian-12-r7` | Sentinel image tag. |
| `redis.metrics.image.repository` | `bitnamilegacy/redis-exporter` | Metrics exporter image. |
| `redis.metrics.image.tag` | `1.58.0-debian-12-r4` | Metrics exporter image tag. |
| `redis.volumePermissions.image.repository` | `bitnamilegacy/os-shell` | Volume permissions image. |
| `redis.volumePermissions.image.tag` | `12-debian-12-r16` | Volume permissions image tag. |
| `redis.sysctl.image.repository` | `bitnamilegacy/os-shell` | Sysctl image. |
| `redis.sysctl.image.tag` | `12-debian-12-r16` | Sysctl image tag. |
| `redis.kubectl.image.repository` | `bitnamilegacy/kubectl` | Kubectl image used by the subchart. |
| `redis.kubectl.image.tag` | `1.29.2-debian-12-r3` | Kubectl image tag. |
| `redis.coordination.enabled` | `true` | Render `general_settings.coordination_redis` into the proxy config. False keeps the bundled Redis for response caching only. A `coordination_redis` block written in `proxy_config` always wins. |

Other values under `redis` pass straight to the Bitnami subchart. To use an existing Redis instead, leave `redis.enabled` false and pass `REDIS_HOST`, `REDIS_PORT`, `REDIS_PASSWORD` or `REDIS_URL` through `envVars` or `environmentSecrets`.

### Migrations Job

| Key | Default | Meaning |
| --- | --- | --- |
| `migrationJob.enabled` | `true` | Run the migrations Job. |
| `migrationJob.retries` | `3` | Retry count documented in `values.yaml`. No template reads it; the Job's retry budget is `backoffLimit`. |
| `migrationJob.backoffLimit` | `4` | Job restarts before the Job fails. |
| `migrationJob.activeDeadlineSeconds` | `1800` | Wall-clock budget for the whole Job across every retry. `null` removes the limit. |
| `migrationJob.disableSchemaUpdate` | `false` | Documented as skipping the migration. No template reads it, so the Job runs migrations either way. |
| `migrationJob.serviceAccountName` | `""` | ServiceAccount for the Job when Helm hooks are enabled and the chart creates a ServiceAccount. Empty uses `default`. |
| `migrationJob.annotations` | `{}` | Annotations on the Job pod. |
| `migrationJob.ttlSecondsAfterFinished` | `120` | Deletion delay for a finished Job. |
| `migrationJob.resources` | `{}` | CPU and memory for the Job container. |
| `migrationJob.extraContainers` | `[]` | Extra containers on the Job pod. |
| `migrationJob.extraInitContainers` | `[]` | Extra init containers on the Job pod. |
| `migrationJob.hooks.argocd.enabled` | `true` | Add the ArgoCD PreSync hook annotations to the Job. |
| `migrationJob.hooks.helm.enabled` | `false` | Run the Job as a `pre-install,pre-upgrade` Helm hook. |
| `migrationJob.hooks.helm.weight` | `1` | Hook weight when the Helm hook is enabled. |

## Database and migrations

The migrations Job runs `betterlite-db-admin migrate` with the chart image. While it is enabled the gateway container gets `DISABLE_SCHEMA_UPDATE=true`, so replicas never race on the schema at startup. The Job sets `DISABLE_SCHEMA_UPDATE=false` for itself, and it is the only container where the variable is false. With `migrationJob.enabled=false` the gateway runs its own startup schema update instead.

The Job is part of the release by default. With `migrationJob.hooks.helm.enabled=true` it runs as a `pre-install,pre-upgrade` Helm hook with a `before-hook-creation` delete policy, and `migrationJob.hooks.argocd.enabled=true`, the default, adds the ArgoCD `PreSync` annotations.

`db.deployStandalone` creates a PostgreSQL instance from the Bitnami subchart and points the Job and the gateway at it. `db.useExisting` points both at a database you run: credentials come from `db.secret`, the host from `db.endpoint` or `db.secret.endpointKey`, and the URL from `db.url`. Set `db.deployStandalone=false` together with `db.useExisting=true`, or the bundled PostgreSQL is still deployed.

`db.connectionPool.enabled` runs PgBouncer inside the pod, so a pod holds at most `db.connectionPool.maxDbConnections` upstream connections no matter how many workers it runs.

## Admin UI

The gateway serves the Admin UI under `/ui` on the same port as the API, `service.port` (4000 by default). Reach it through whatever publishes the Service: an Ingress enabled with `ingress.*`, a LoadBalancer, or `kubectl port-forward`. Sign in with the master key from `masterkey` or from the Secret named by `masterkeySecretName`.

## Autoscaling

`autoscaling.enabled` renders an autoscaling/v2 HorizontalPodAutoscaler and removes `replicas` from the Deployment, so `replicaCount` no longer applies. The default metric is CPU utilization at 60 percent. Add a memory target only through `autoscaling.targetMemoryUtilizationPercentage`, because the query engine's memory use only ratchets up. `autoscaling.targetRequestsPerSecond` and `autoscaling.targetTokensPerSecond` add Pods metrics and need a Prometheus Adapter serving `litellm_requests_per_second` and `litellm_tokens_per_second`.

`keda.enabled` renders a ScaledObject instead, and it renders only while `autoscaling.enabled` is false; the two are mutually exclusive. `keda.triggers` takes arbitrary KEDA triggers. `keda.prometheus.requestsPerSecond` and `keda.prometheus.tokensPerSecond` add Prometheus triggers on the gateway's own request and token counters and require `keda.prometheus.serverAddress`.

## Tests

The chart's suites live under `tests/` and run with helm-unittest. Every check goes through `packaging/helm/tests/helm_checks.sh`, which copies the chart to a temporary folder and builds its dependencies there, so nothing is written into the repository. Run the checks from the repository root:

```bash
sh packaging/helm/tests/helm_checks.sh lint packaging/helm/betterlite-helm packaging/helm/betterlite-helm/ci/test-values.yaml
sh packaging/helm/tests/helm_checks.sh unittest packaging/helm/betterlite-helm tests/deployment_tests.yaml
```

The unittest mode takes one or more suite files, named relative to the chart. `packaging/helm/tests/check_suites.sh` runs every `tests/*_tests.yaml` file of both charts and fails when the number of suites helm-unittest ran differs from the number of suite files. The checks need Helm 3 and the helm-unittest plugin.
