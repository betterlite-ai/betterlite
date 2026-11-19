# betterlite on GCP (Cloud Run)

This module deploys betterlite on GCP: one Cloud Run service behind an external load balancer, Cloud SQL for PostgreSQL with a read replica, Memorystore for Redis, two GCS buckets, Secret Manager entries and a migration job that runs before the service starts.

The module has no provider block, so the caller owns the provider configuration. `examples/default/` is a runnable root that sets `google` to the project and region and calls the module with a curated set of inputs.

## What the module builds

- A VPC with a subnet, a private services access range and a Serverless VPC Access connector, or nothing when `network_id` names a network you already run.
- Cloud SQL for PostgreSQL: a writer instance, a cross-zone read replica, the database and an application user whose password is generated and stored in Secret Manager.
- Memorystore for Redis on a private IP with AUTH enabled.
- One Cloud Run service running betterlite on port 4000. Its ingress accepts traffic from the load balancer only, and `/health/liveliness` and `/health/readiness` serve the Cloud Run probes.
- A Cloud Run migration job that runs `betterlite-db-admin migrate` against the writer before the service starts.
- An external global HTTP(S) load balancer: a global anycast address, a serverless NEG, a backend service whose timeout follows `timeout_seconds`, and, with `lb_domains`, a Google-managed certificate on port 443 with HTTP redirected to HTTPS.
- Two GCS buckets: a versioned private data bucket exported to the service as `GCS_BUCKET_NAME`, and a config bucket that exists only when `proxy_config` has keys.
- Secret Manager entries for the master key and the database password, plus the license and UI password when they are set. The runtime service account gets `roles/cloudsql.client`, `roles/logging.logWriter`, `objectAdmin` on the data bucket, `objectViewer` on the config bucket and secret access to each entry.

Every resource is named `<tenant>-<env>-betterlite` plus a per-resource suffix, cut where GCP caps a name (30 characters for a service account id, 25 for a VPC connector). Two deployments can share a project as long as the `(tenant, env)` pair differs.

## Quick start

1. Pick a GCP project, enable billing and the APIs: `run`, `sqladmin`, `redis`, `secretmanager`, `vpcaccess`, `compute`, `servicenetworking` and `storage`. Log in with `gcloud auth login` on the machine that runs Terraform, because the apply executes the migration job through gcloud.

2. Copy the sample variables file and fill in `project_id`, `image` and `litellm_master_key`:

   ```bash
   cd packaging/terraform/gcp/examples/default
   cp terraform.tfvars.example terraform.tfvars
   ```

3. Initialize the root. The lock file pins the providers for `linux_amd64`, `linux_arm64` and `darwin_arm64`:

   ```bash
   terraform init
   ```

4. Apply. Without `lb_domains` the load balancer has no certificate, so a trial deploy also sets `allow_plaintext_lb = true`; see [TLS](#tls).

   ```bash
   terraform apply
   ```

5. Read the URL and call the health route:

   ```bash
   terraform output -raw lb_url
   curl "$(terraform output -raw lb_url)/health/liveliness"
   ```

The first apply takes about 20 minutes, mostly Cloud SQL. The migration job runs when the database is ready, and only then does Cloud Run start the service.

## Inputs

The module's inputs live in three files. Sizes follow GCP's units: `db_tier` is a Cloud SQL machine type, `redis_memory_size_gb` is gigabytes, `cpu` is a Cloud Run CPU count and `memory` a Cloud Run memory string.

### variables_data.tf

| Name | Description | Type | Default |
| --- | --- | --- | --- |
| `project_id` | GCP project that holds the network, the data stores and their secrets. | `string` | required |
| `region` | GCP region for the VPC, Cloud SQL, Memorystore and the serverless connector. | `string` | `"us-central1"` |
| `tenant` | Tenant slug, the first part of every resource name. Combined with env it forms `<tenant>-<env>-betterlite`. | `string` | `"demo"` |
| `env` | Environment slug, the second part of every resource name. | `string` | `"dev"` |
| `labels` | Labels applied to every label-supporting resource, on top of the module's `managed_by` label. | `map(string)` | `{}` |
| `create_runtime` | Create the runtime side of the module. Set false to build only the network and the data stores for a caller that runs the service elsewhere. | `bool` | `true` |
| `network_id` | Existing VPC network resource id (`projects/<project>/global/networks/<name>`). When set, no VPC or subnet is created. | `string` | `""` |
| `create_psa_connection` | Create the private services access range and connection that Cloud SQL and Memorystore need. Set false when the existing network already has private services access. | `bool` | `true` |
| `subnet_cidr` | Primary CIDR block for the betterlite subnet. Unused when `network_id` is set. | `string` | `"10.20.0.0/24"` |
| `vpc_connector_cidr` | CIDR block for the serverless VPC Access connector, a `/28`. Unused when `create_runtime` is false. | `string` | `"10.20.1.0/28"` |
| `db_tier` | Cloud SQL machine type for the writer and the reader. | `string` | `"db-custom-2-7680"` |
| `db_edition` | Cloud SQL edition. `ENTERPRISE` accepts the `db-custom-*` tiers; `ENTERPRISE_PLUS` only accepts `db-perf-optimized-*` tiers. | `string` | `"ENTERPRISE"` |
| `db_version` | Cloud SQL Postgres version. | `string` | `"POSTGRES_16"` |
| `db_name` | Database created on the Cloud SQL writer. | `string` | `"betterlite"` |
| `db_username` | Application Postgres user. Its password is generated and stored in Secret Manager. | `string` | `"betterlite"` |
| `cloudsql_deletion_protection` | Cloud SQL instance deletion protection on the writer and the reader. Set false only for ephemeral environments. | `bool` | `true` |
| `redis_tier` | Memorystore tier: `STANDARD_HA` for production, `BASIC` for development. | `string` | `"STANDARD_HA"` |
| `redis_memory_size_gb` | Memorystore capacity in GB. | `number` | `1` |
| `redis_transit_encryption` | Enable Memorystore in-transit encryption. The client then needs the server CA, which betterlite cannot read yet, so this defaults to false. | `bool` | `false` |
| `create_redis` | Create the Memorystore instance. Set false to use a Redis the caller runs. | `bool` | `true` |

### variables_identity.tf

| Name | Description | Type | Default |
| --- | --- | --- | --- |
| `litellm_master_key` | Master key betterlite accepts as `LITELLM_MASTER_KEY` (an `sk-...` string). Required, because betterlite does not generate one for you. Sensitive. | `string` | required |
| `litellm_license` | LiteLLM license string. When set, the module creates a license secret and grants the runtime account access to it. Empty leaves it out for an OSS deployment. Sensitive. | `string` | `""` |
| `ui_password` | Admin UI password. When set, the module creates a `ui-password` secret and grants the runtime account access to it. Empty logs the UI in with the master key. Sensitive. | `string` | `""` |
| `extra_secrets` | Environment variable name to the Secret Manager secret id (`projects/<project>/secrets/<name>`) of a secret the caller created. The runtime account gets secretAccessor on each distinct secret. | `map(string)` | `{}` |
| `proxy_config` | betterlite proxy config, the same YAML-shaped map the Helm chart and the AWS module take. When it has keys the module uploads it as `config.yaml` to a dedicated bucket. | `any` | `{}` |
| `bucket_force_destroy` | Let `terraform destroy` delete the data and config buckets while they still hold objects. Set true only for ephemeral environments. | `bool` | `false` |
| `bucket_location` | GCS location for both buckets, a region or a multi-region such as `EU`. Empty means `region`. | `string` | `""` |
| `data_retention_days` | Days after which the data bucket deletes its objects through a lifecycle rule. `0` means no rule, so objects stay until someone removes them. | `number` | `0` |

### variables_service.tf

| Name | Description | Type | Default |
| --- | --- | --- | --- |
| `image` | Container image for the betterlite Cloud Run service and the migration job. B5071 ships betterlite and betterlite-db-admin in one image. | `string` | required |
| `cpu` | CPU limit for the Cloud Run container. Cloud Run accepts 1, 2, 4 and 8. | `string` | `"2"` |
| `memory` | Memory limit for the Cloud Run container. | `string` | `"4Gi"` |
| `min_instances` | Minimum number of Cloud Run instances kept running. | `number` | `1` |
| `max_instances` | Maximum number of Cloud Run instances. | `number` | `10` |
| `concurrency` | Maximum concurrent requests each Cloud Run instance accepts. | `number` | `80` |
| `timeout_seconds` | Request timeout in seconds for the Cloud Run service and the load balancer backend service. | `number` | `300` |
| `lb_domains` | DNS names that point at `lb_ip`. When set, the module adds a Google-managed certificate and an HTTPS forwarding rule and redirects HTTP to HTTPS. | `list(string)` | `[]` |
| `allow_plaintext_lb` | Serve plain HTTP on the load balancer when `lb_domains` is empty. Use only for a trial or a development environment. | `bool` | `false` |
| `extra_env` | Additional environment variables for the service. A name the module also sets replaces the module's value. | `map(string)` | `{}` |
| `connection_pool_enabled` | Route the service through a PgBouncer connection pool in front of Cloud SQL. | `bool` | `false` |
| `pool_max_db_connections` | Maximum database connections the PgBouncer pool opens. | `number` | `20` |
| `pool_max_client_conn` | Maximum client connections the PgBouncer pool accepts. | `number` | `1000` |
| `otel_endpoint` | OTLP endpoint for OpenTelemetry traces and metrics. Empty leaves OpenTelemetry off. | `string` | `""` |

## Outputs

| Name | Description |
| --- | --- |
| `lb_ip` | Global anycast IP of the external load balancer, or null when `create_runtime` is false. |
| `lb_url` | Proxy URL: `https://` with the first `lb_domains` entry, `http://` with the load balancer IP when plaintext is allowed, or null without the runtime. |
| `service_url` | Default Cloud Run URL of the betterlite service, or null when `create_runtime` is false. |
| `cloud_run_service_name` | Name of the betterlite Cloud Run service, or null when `create_runtime` is false. |
| `runtime_service_account` | Email of the service account the Cloud Run service and the migration job run as. |
| `writer_private_ip` | Private IP of the Cloud SQL writer. |
| `reader_private_ip` | Private IP of the Cloud SQL read replica. |
| `redis_host` | Memorystore Redis host, or null when `create_redis` is false. |
| `data_bucket` | GCS bucket betterlite keeps its data in, exported to the service as `GCS_BUCKET_NAME`. |
| `config_bucket` | GCS bucket holding `config.yaml`, or null when no `proxy_config` is set. |
| `master_key_secret_id` | Secret Manager resource id holding `LITELLM_MASTER_KEY`. |
| `db_password_secret_id` | Secret Manager resource id holding the Cloud SQL application user's password. |
| `network_id` | Resource id of the VPC network the stores and the runtime attach to. |
| `migration_job_name` | Name of the Cloud Run migration job, or null when `create_runtime` is false. |

## Deps-only mode

`create_runtime = false` builds the network, Cloud SQL, Memorystore, the buckets and the Secret Manager entries only. No Cloud Run service, no migration job, no load balancer, no Serverless VPC Access connector, and no config bucket. The runtime outputs `lb_ip`, `lb_url`, `service_url`, `cloud_run_service_name` and `migration_job_name` are null.

Use it when the service runs somewhere else in the same network, such as GKE. Run the migration from whichever runtime you deploy: the module's gcloud step exists only with `create_runtime = true`.

## Bring your own network

Set `network_id` to the full network resource id to skip the VPC and subnet, which also makes `subnet_cidr` unused. Set `create_psa_connection = false` when that network already has private services access for Cloud SQL and Memorystore; the host project owns that peering.

With `create_runtime = true` the module still creates the Serverless VPC Access connector, and a connector has to live in the same project as the network, so that combination only works when the VPC is in the deployment project. For a shared VPC in another project, run deps-only mode here and deploy the service from a root in the host project.

## TLS

The load balancer is HTTPS by default. List DNS names in `lb_domains` and the module adds a Google-managed certificate, a 443 forwarding rule and a port-80 rule that answers with a permanent redirect to HTTPS. The names have to point at `lb_ip`, so the usual first pass is:

1. Apply with `allow_plaintext_lb = true` and `lb_domains = []` to get the address, `terraform output -raw lb_ip`.
2. Point an `A` record for each name at that address.
3. Set `lb_domains = ["proxy.example.com"]`, remove `allow_plaintext_lb` and apply again.

Without `lb_domains` the plan fails unless `allow_plaintext_lb = true`, and then port 80 serves the service directly with no certificate. That is meant for a trial or a development environment.

A managed certificate stays in `PROVISIONING` for 15 to 60 minutes after DNS resolves, and the load balancer cannot complete a TLS handshake before it turns `ACTIVE`. Watch it with `gcloud compute ssl-certificates list` and `gcloud compute ssl-certificates describe <name>`; the certificate is named `-cert-` plus a hash of the domain list, so `list` is the quicker way to find it.

## Migration order

The service starts only after the schema migration succeeded. The module creates a Cloud Run job named `<tenant>-<env>-betterlite-migrate` that runs the same image with command `betterlite-db-admin` and argument `migrate`, with `max_retries = 0`, the writer's host, port, user and database, and the password secret. A `terraform_data` resource then runs

```bash
gcloud run jobs execute <job> --region <region> --project <project> --wait
```

in a local-exec provisioner, and the Cloud Run service depends on it. The apply therefore finishes only when the job exits successfully, and a failed migration fails the apply and leaves the service uncreated.

Two consequences: the machine running Terraform needs the gcloud CLI installed and logged in (`gcloud auth login`) in addition to its Terraform credentials, and the provisioner re-runs when the job id or the image changes. To re-run the migration by hand, execute the same command, or read the job name from `terraform output -raw migration_job_name` in the example root.

## Connection pool

`connection_pool_enabled = true` adds three variables to the Cloud Run service: `LITELLM_PGBOUNCER_ENABLED=true`, `LITELLM_PGBOUNCER_MAX_DB_CONNECTIONS` from `pool_max_db_connections` and `LITELLM_PGBOUNCER_MAX_CLIENT_CONN` from `pool_max_client_conn`. The migration job always connects straight to the writer.

betterlite reads `LITELLM_PGBOUNCER_ENABLED` and `LITELLM_PGBOUNCER_MAX_DB_CONNECTIONS` and logs that it does not launch an embedded PgBouncer yet, so the pool takes effect only once the gateway runs one; the client-connection limit is read and ignored today. The variables are wired now so the deployment does not have to change then, and the defaults (`20` upstream connections, `1000` client connections) match betterlite's.

## Redis transit encryption

`redis_transit_encryption` defaults to `false`, which creates the Memorystore instance with `transit_encryption_mode = DISABLED` and gives the service plain `REDIS_HOST` and `REDIS_PORT`. AUTH stays on either way, so an instance inside the private services range is not open to every client that can reach it.

Setting it to `true` switches the instance to `SERVER_AUTHENTICATION` and adds `REDIS_SSL=true` to the service. Memorystore then presents a self-signed server certificate, and nothing passes its CA to the service, so the client cannot validate it and the connection fails the handshake. Leave the variable off until betterlite can be given the instance's CA file; the module's variable description says the same.

## Not built compared with LiteLLM

LiteLLM's GCP module deploys three Cloud Run services (gateway, backend and UI) and has no single-service mode. Betterlite serves the API, the management routes and the UI from one process, so this module creates one service and no path-routing URL map.

These LiteLLM pieces have no counterpart here:

- The `spend-collector` sidecar and its `collector_*` inputs.
- The Prometheus metrics sidecar and `gateway_metrics_port`.
- The enterprise billing metrics.
- Separate backend and UI services with their own images, ports and sizes.

The one service also means the image is a single input; there is no per-component image override.

## Tests

The module ships plan-only tests that run against mock providers, so they need no GCP project or credentials:

```bash
terraform -chdir=packaging/terraform/gcp init
terraform -chdir=packaging/terraform/gcp test
```

The files under `packaging/terraform/gcp/tests/` cover the default stack, deps-only mode, a caller-provided network, the load balancer in plaintext and TLS modes, the service environment and secrets, and the connection pool.

The example root is checked separately: `terraform -chdir=packaging/terraform/gcp/examples/default fmt -check`, `init -backend=false -lockfile=readonly` and `validate`.
