# betterlite Terraform modules

`packaging/terraform/` holds two Terraform modules that deploy betterlite, one for AWS and one for GCP. Each has no `provider` block, so the caller owns the region, the credentials and any provider aliases, and each ships a runnable root under `examples/default/`. The two build the same stack from the same input names, on the managed services each cloud offers.

## Modules

`aws/` deploys betterlite on ECS Fargate behind an Application Load Balancer, with Aurora PostgreSQL, ElastiCache Redis, an S3 bucket, Secrets Manager entries and a schema migration that runs before the service starts. The [AWS module README](aws/README.md) covers its inputs, outputs, the bring-your-own network and store paths, TLS and autoscaling.

`gcp/` deploys betterlite on Cloud Run behind an external global load balancer, with Cloud SQL for PostgreSQL, Memorystore for Redis, GCS buckets, Secret Manager entries and a Cloud Run migration job. The [GCP module README](gcp/README.md) covers its inputs, outputs, deps-only mode, TLS and the connection pool.

## What both build

- One betterlite container on port 4000, serving the data plane, the management routes and the admin UI from one process.
- A PostgreSQL database with a read replica: Aurora on AWS, Cloud SQL on GCP.
- A Redis cache for rate limits, budgets and router cooldowns shared across instances: ElastiCache on AWS, Memorystore on GCP.
- A bucket holding the proxy config. When `proxy_config` has keys, the module uploads it as `config.yaml` and points the service at it, in the stack's S3 bucket on AWS and in a dedicated config bucket on GCP.
- Object storage for the service's data: an S3 bucket on AWS, a GCS data bucket on GCP.
- The master key and the database credentials in the cloud's secret manager, plus a secret for the license and the UI password when they are set: Secrets Manager on AWS, Secret Manager on GCP.
- An HTTPS load balancer in front of the service, using the ACM certificate you pass on AWS and a Google-managed certificate on GCP once `lb_domains` is set. Plain HTTP needs `allow_plaintext_alb` or `allow_plaintext_lb`.
- The schema migration, run before the service starts: a one-off ECS task on AWS, a Cloud Run job on GCP.

## Choosing a module

Pick the cloud the rest of the stack already runs in. The deployment is the same on both; the services under it differ:

| Role | AWS | GCP |
| --- | --- | --- |
| Compute | ECS Fargate | Cloud Run |
| Database | Aurora PostgreSQL | Cloud SQL for PostgreSQL |
| Cache | ElastiCache for Redis | Memorystore for Redis |
| Load balancer | Application Load Balancer | Global external load balancer |
| Object store | S3 | Cloud Storage |
| Secrets | Secrets Manager | Secret Manager |

## Shared input names

Both modules declare these inputs with the same meaning on either cloud:

| Name | What it sets |
| --- | --- |
| `tenant` | Tenant slug, the first part of every resource name. |
| `env` | Environment slug, appended to every resource name. |
| `image` | Container image for the service and the migration job or task. |
| `litellm_master_key` | Master key betterlite accepts as `LITELLM_MASTER_KEY`. AWS generates one when it is empty; GCP requires it. |
| `litellm_license` | License exposed as `LITELLM_LICENSE`; empty leaves the secret out. |
| `ui_password` | Admin UI password; empty logs the UI in with the master key. |
| `extra_secrets` | Map of environment variable name to a secret the caller created: a Secrets Manager ARN on AWS, a Secret Manager resource id on GCP. The service gets read access to each. |
| `proxy_config` | Proxy config as a YAML-shaped map. When it has keys, the module uploads it as `config.yaml` and points the service at it. |
| `extra_env` | Extra plain-text environment variables. A name the module also sets replaces the module's value. |
| `connection_pool_enabled` | Set `LITELLM_PGBOUNCER_ENABLED` and both limits on the service, for a PgBouncer pool in front of PostgreSQL. |
| `pool_max_db_connections` | Upstream Postgres connections the pool may hold, 20 by default. |
| `pool_max_client_conn` | Client connections the pool accepts, 1000 by default. |
| `otel_endpoint` | OTLP endpoint for traces and metrics; empty leaves OpenTelemetry off. |

Beyond these, each module has the inputs its cloud needs, such as the project, network, database sizing and TLS settings listed in each module README.

## Differences

- Database authentication. The AWS service connects as an application user with the `rds_iam` role and mints short-lived IAM tokens, with `IAM_TOKEN_DB_AUTH=true`, so no database password reaches the container. The GCP service connects with a generated password for the application user, stored in Secret Manager and passed as `DATABASE_PASSWORD`.
- Migration runner. The AWS module starts the bootstrap and migration as one-off ECS tasks and waits on them with the AWS CLI (`aws ecs run-task`, `aws ecs wait`). The GCP module runs a Cloud Run migration job with the gcloud CLI (`gcloud run jobs execute --wait`). Either way, the machine that runs `terraform apply` needs that CLI installed and authenticated.
- Redis TLS default. ElastiCache runs with in-transit encryption on, so the AWS service always gets `REDIS_SSL=true`. Memorystore's `redis_transit_encryption` defaults to false, because betterlite cannot validate Memorystore's server certificate yet. Setting it true adds `REDIS_SSL=true`, and the connection then fails the handshake until betterlite can be given the CA.

## Tools

Terraform 1.13 or newer. Both modules pin their providers in a committed `.terraform.lock.hcl`, so `terraform init` runs with `-lockfile=readonly`. The machine that runs `terraform apply` also needs the `aws` CLI on AWS or the `gcloud` CLI on GCP, installed and authenticated, because the migration step runs through it. The plan-only tests under each module's `tests/` folder mock the providers, so they need no cloud account and no credentials.

## Not built compared with LiteLLM

LiteLLM's [Terraform modules](https://github.com/BerriAI/litellm/tree/d665dc1a6315847fcad6cc2c1264ceb47c06abce/terraform/litellm) deploy the proxy as three services, gateway, backend and ui, with sidecars beside them. betterlite serves the data plane, the management routes and the admin UI from one process, so these parts of LiteLLM's stack are absent by design:

- The collector sidecar and its `collector_*` inputs.
- The Prometheus metrics sidecar and the port it exposes.
- The enterprise billing metrics.
- The separate backend and ui services, with their own images, ports and sizes. One image runs one service on port 4000.
