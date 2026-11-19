# betterlite on AWS (ECS Fargate)

The module in this folder deploys betterlite on ECS Fargate: an Aurora PostgreSQL database, an ElastiCache Redis, one service behind an Application Load Balancer, and the S3 and Secrets Manager entries the server reads. It declares no `provider` block, so the caller owns the region, the credentials and any assume-role or aliased provider. `examples/default/` is the runnable root: it configures the `aws` provider, calls the module with `source = "../.."`, and is where a person runs `terraform apply`.

Every resource the stack creates is named `${tenant}-litellm-${env}` or that plus a per-resource suffix, so several tenants and environments can share an account as long as the pair differs.

## What the module builds

- A VPC with public and private subnets across the AZs you pass, an internet gateway, one NAT gateway and route tables, skipped when you set `vpc_id`.
- An Aurora Postgres cluster (one writer and one reader instance) with IAM database authentication and storage encryption enabled, skipped when `create_database = false`.
- An ElastiCache Redis replication group, private, with at-rest and in-transit encryption, automatic failover and multi-AZ when `redis_num_replicas` is at least 1, skipped when `create_redis = false`.
- An S3 bucket, versioned, encrypted with SSE-S3 and with public access blocked, exposed to the container as `S3_BUCKET_NAME` and `S3_REGION_NAME` for the config file, cached responses, archived request logs and file uploads.
- Secrets Manager entries for `LITELLM_MASTER_KEY` (generated as an `sk-` key when you do not pass one), the license, the UI password, the Aurora master password (read only by the bootstrap task) and the URL of an existing database or cache.
- An ECS Fargate cluster with Container Insights and one service: one container listening on port 4000, its logs in a CloudWatch group, and an IAM task role that may mint database tokens and read the bucket.
- An Application Load Balancer with a target group whose health check polls `/health/readiness`, port 80 forward or redirect, and a 443 listener when you pass a certificate.
- Application Auto Scaling target tracking on CPU and memory, plus an optional requests-per-target policy.
- Two one-off ECS tasks, run by `terraform apply` through the AWS CLI: the database bootstrap and the schema migration.

## Quick start

1. Copy the sample variables and keep your own file out of version control.

   ```bash
   cd packaging/terraform/aws/examples/default
   cp terraform.tfvars.example terraform.tfvars
   ```

2. Set `image` to a pushed betterlite image and replace `litellm_master_key` (or export `TF_VAR_litellm_master_key`). To try the stack over plain HTTP, also set `allow_plaintext_alb = true`; otherwise set `acm_certificate_arn`.

3. Initialize the folder against the committed lock file.

   ```bash
   terraform init
   ```

4. Apply. The machine running Terraform needs the `aws` CLI installed and authenticated, because the bootstrap and migration tasks start through `aws ecs run-task`. That one apply creates the infrastructure, creates the IAM-authenticated database user, runs the schema migration, and starts the service last.

   ```bash
   terraform apply
   ```

5. Read the URL and check that the service answers its readiness probe.

   ```bash
   curl "$(terraform output -raw alb_url)/health/readiness"
   ```

The master key is in Secrets Manager if you let the module generate it:

```bash
aws secretsmanager get-secret-value \
  --secret-id "$(terraform output -raw master_key_secret_arn)" \
  --query SecretString --output text
```

## Inputs

### variables_data.tf

| Name | Type | Default | Description |
| --- | --- | --- | --- |
| `region` | `string` | required | AWS region the module deploys into. |
| `tenant` | `string` | required | Tenant slug, the first part of every resource name. 1 to 21 lower-case letters, digits and hyphens, starting with a letter. |
| `env` | `string` | required | Environment slug appended to every resource name. 1 to 9 lower-case letters, digits and hyphens, starting with a letter. |
| `tags` | `map(string)` | `{}` | Extra tags applied to every taggable resource, merged over the module's `litellm:stack` and `managed-by` tags. |
| `vpc_id` | `string` | `""` | Existing VPC to deploy into. Empty has the module create the VPC, subnets, NAT gateway and route tables. |
| `public_subnet_ids` | `list(string)` | `[]` | Existing public subnets for the load balancer. Required when `vpc_id` is set, ignored otherwise. |
| `private_subnet_ids` | `list(string)` | `[]` | Existing private subnets for the tasks, Aurora and ElastiCache. Required when `vpc_id` is set, ignored otherwise. |
| `additional_task_security_group_ids` | `list(string)` | `[]` | Extra security groups attached to the tasks, for a database or cache that already allows one of yours. |
| `vpc_cidr` | `string` | `"10.40.0.0/16"` | CIDR block for the VPC the module creates. Unused when `vpc_id` is set. |
| `azs` | `list(string)` | `[]` | Availability zones for the created subnets, at least 2. Unused when `vpc_id` is set. |
| `create_database` | `bool` | `true` | Create the Aurora Postgres cluster. Set false to use `database_url` or run without a database. |
| `database_url` | `string` sensitive | `""` | Postgres connection string for an existing database, read only when `create_database = false`. |
| `db_instance_class` | `string` | `"db.r6g.large"` | Aurora instance class for the writer and reader. |
| `db_engine_version` | `string` | `"16.4"` | Aurora Postgres engine version; its major version selects the parameter group family. |
| `db_name` | `string` | `"litellm"` | Initial database created on the cluster. |
| `db_master_username` | `string` | `"postgres"` | Aurora master username, used only to bootstrap the application user. |
| `db_username` | `string` | `"litellm_app"` | Postgres user the tasks connect as, created with the `rds_iam` role. |
| `create_redis` | `bool` | `true` | Create the ElastiCache Redis replication group. Set false to use `redis_url` or run without Redis. |
| `redis_url` | `string` sensitive | `""` | Redis connection string for an existing cache, read only when `create_redis = false`. |
| `redis_node_type` | `string` | `"cache.t4g.small"` | ElastiCache node type. |
| `redis_num_replicas` | `number` | `1` | Read replicas in the replication group. At least 1 turns on automatic failover and multi-AZ. |
| `skip_final_snapshot` | `bool` | `false` | Skip the Aurora final snapshot on destroy. Default false takes the snapshot. |

### variables_identity.tf

| Name | Type | Default | Description |
| --- | --- | --- | --- |
| `litellm_master_key` | `string` sensitive | `""` | Master key written to the master-key secret verbatim. Empty has the module generate an `sk-` key. |
| `litellm_license` | `string` sensitive | `""` | Enterprise license exposed as `LITELLM_LICENSE`. Empty creates no license secret. |
| `ui_password` | `string` sensitive | `""` | Admin UI password exposed as `UI_PASSWORD`. Empty logs in with the master key. |
| `extra_secrets` | `map(string)` | `{}` | Map of environment variable name to secret ARN; the execution role gains read access to each one. |
| `otel_headers_secret_arn` | `string` | `""` | Secret ARN whose plaintext value becomes `OTEL_HEADERS`. Empty omits the variable. |
| `proxy_config` | `any` | `{}` | Proxy config (the contents of config.yaml), uploaded to the bucket as YAML and read by the service at startup. Empty uploads nothing. |
| `s3_force_destroy` | `bool` | `false` | Allow `terraform destroy` to delete the bucket while it still holds objects. |
| `log_retention_days` | `number` | `30` | CloudWatch log retention in days for the stack's log groups. |

### variables_service.tf

| Name | Type | Default | Description |
| --- | --- | --- | --- |
| `image` | `string` | required | Container image running the betterlite server. There is no published image name yet, so the caller supplies one. |
| `cpu` | `number` | `1024` | Fargate CPU units for the task, where 1024 is one vCPU. |
| `memory` | `number` | `4096` | Fargate memory in MiB. |
| `desired_count` | `number` | `2` | Tasks to start with. Application Auto Scaling owns the count afterwards when autoscaling is on. |
| `autoscaling_enabled` | `bool` | `true` | Turn on Application Auto Scaling target tracking for the service. |
| `min_capacity` | `number` | `1` | Minimum task count under autoscaling. |
| `max_capacity` | `number` | `10` | Maximum task count under autoscaling. |
| `cpu_target` | `number` | `70` | Target average CPU utilization in percent. |
| `memory_target` | `number` | `80` | Target average memory utilization in percent. Set 0 to scale on CPU only. |
| `target_requests_per_second` | `number` | `0` | Requests per second one task should serve, added as an `ALBRequestCountPerTarget` policy. 0 skips the policy. |
| `connection_pool_enabled` | `bool` | `false` | Run a PgBouncer connection pool inside the task. Needs a database. |
| `pool_max_db_connections` | `number` | `20` | Upstream Postgres connections the task may hold when the pool is on. |
| `pool_max_client_conn` | `number` | `1000` | Client connections the in-task PgBouncer accepts when the pool is on. |
| `extra_env` | `map(string)` | `{}` | Additional plain-text environment variables. A name the module already sets replaces the module's value. |
| `acm_certificate_arn` | `string` | `""` | ACM certificate ARN for the ALB's HTTPS listener. Empty serves HTTP only, which also needs `allow_plaintext_alb`. |
| `allow_plaintext_alb` | `bool` | `false` | Serve the ALB over HTTP without TLS. Meant for short-lived trial and dev stacks. |
| `otel_endpoint` | `string` | `""` | OTLP collector endpoint, which turns on the OpenTelemetry v2 environment block. Empty disables OTel. |
| `otel_exporter` | `string` | `"otlp_http"` | OTLP exporter protocol: `otlp_http`, `otlp_grpc` or `console`. |
| `otel_environment_name` | `string` | `""` | Value for `OTEL_ENVIRONMENT_NAME`. Empty falls back to `env`. |
| `otel_capture_message_content` | `string` | `"no_content"` | Value for `OTEL_INSTRUMENTATION_GENAI_CAPTURE_MESSAGE_CONTENT`: `no_content` or `prompt_and_completion`. |

## Outputs

| Name | Description |
| --- | --- |
| `alb_dns_name` | Public DNS name of the load balancer. |
| `alb_url` | Proxy URL, `https` when a certificate is set, `http` otherwise. The dashboard is served at `/` and the API at `/v1/*`. |
| `ecs_cluster` | ECS cluster name. |
| `vpc_id` | VPC the stack runs in, whether the module created it or the caller passed it in. |
| `task_security_group_id` | Security group attached to the tasks. Allow inbound from this group on an existing database or Redis. |
| `aurora_writer_endpoint` | Aurora writer endpoint, which the container takes as `DATABASE_HOST`. Null when `create_database = false`. |
| `aurora_reader_endpoint` | Aurora reader endpoint, which the container takes as `DATABASE_HOST_READ_REPLICA`. Null when `create_database = false`. |
| `redis_endpoint` | ElastiCache primary endpoint as `host:port`. Null when `create_redis = false`. |
| `s3_bucket` | Bucket name, exposed to the container as `S3_BUCKET_NAME`. |
| `master_key_secret_arn` | Secrets Manager ARN holding `LITELLM_MASTER_KEY`. |
| `db_master_password_secret_arn` | Secrets Manager ARN holding the Aurora master credentials, which only the bootstrap task reads. Null when `create_database = false`. |
| `db_bootstrap_sql` | SQL to run once as the master DB user to create the IAM-authenticated application user. Empty when `create_database = false`. |
| `migration_run_command` | Shell command that runs the migration task by hand, for when an apply could not. Empty when the stack has no database. |

## Bring your own network and stores

Networking is optional. Set `vpc_id` together with `public_subnet_ids` and `private_subnet_ids` and the module creates no VPC, subnets, route tables, internet gateway or NAT gateway; `vpc_cidr` and `azs` go unused. The ALB goes into the public subnets, the tasks and any subnet group the module still needs go into the private ones. The private subnets need their own egress, through a NAT gateway or VPC endpoints covering ECR, S3, CloudWatch Logs and Secrets Manager, because the tasks pull images, resolve secrets and call the model providers. Aurora and ElastiCache subnet groups need subnets in at least 2 AZs, so pass 2 private subnets unless both stores are off.

```hcl
vpc_id             = "vpc-0123456789abcdef0"
public_subnet_ids  = ["subnet-aaa", "subnet-bbb"]
private_subnet_ids = ["subnet-ccc", "subnet-ddd"]
```

The example root in `examples/default/` sets `azs` to the first two zones of `var.region`; change that argument in its `main.tf` to place the subnets elsewhere.

Security groups stay module-owned in either mode: the ALB group, the tasks group, and the database and cache groups when the module creates those. To reach infrastructure the module does not manage, allow inbound from the group named by `task_security_group_id`, or attach a group of your own with `additional_task_security_group_ids`.

The data stores are optional in the same way. Set `create_database = false` and pass `database_url`, set `create_redis = false` and pass `redis_url`, or leave the URL empty and run without that store. A supplied URL lands in a Secrets Manager entry and reaches the container as `DATABASE_URL` or `REDIS_URL`, so it never appears in plain text in the task definition, and the migration task still runs against the existing database. The Aurora-specific IAM user bootstrap drops out, since the URL already carries its credentials.

```hcl
create_database = false
database_url    = "postgresql://user:password@db.internal:5432/litellm"
create_redis    = false
redis_url       = "rediss://:password@cache.internal:6379"
```

Without a database there are no virtual keys, teams, spend tracking or persisted UI state, `STORE_MODEL_IN_DB` is not set and requests authenticate with the master key only. Without Redis, rate limits, budgets and router cooldowns are counted per task, and the module warns at plan time when more than one task can run; hold the service to one task or pass a `redis_url`.

## TLS

TLS is the supported posture and plaintext has to be chosen explicitly. A plan fails with a message pointing at the flag when the ALB would have no HTTPS listener.

For production and staging, create or import an ACM certificate in the same region that covers the DNS name you will point at the ALB, then set:

```hcl
acm_certificate_arn = "arn:aws:acm:us-east-1:111122223333:certificate/example"
```

Port 443 then carries the forwarding rule and port 80 answers with a permanent 301 redirect to HTTPS, so HTTP clients are upgraded.

A trial or dev stack can opt into HTTP only:

```hcl
allow_plaintext_alb = true
```

With an empty certificate and `allow_plaintext_alb = false`, the plan stops at the listener with the precondition error.

## Migration

The schema is in place before the service starts, in three steps that one apply runs in order:

1. The bootstrap task runs as the Aurora master user through `postgres:16-alpine`. It creates the application user and grants it `rds_iam`, the role that lets it log in with an IAM token, plus the schema privileges it needs. The SQL is idempotent, so re-applies are safe.
2. The migration task starts the same image with `betterlite-db-admin migrate` as its entry point and exits when the schema is current.
3. The service starts, with a `depends_on` on the migration, and the tasks connect as the application user with short-lived IAM tokens. The task role's `rds-db:connect` grant is scoped to that user on the cluster.

Both one-off tasks are started with `aws ecs run-task` and waited on with `aws ecs wait`, so the applying machine needs the AWS CLI and credentials.

For a database you brought yourself, only the migration step runs: its URL already carries working credentials. If an apply could not run the migration, `migration_run_command` holds the `aws ecs run-task` line for that task, and `db_bootstrap_sql` holds the bootstrap SQL to run by hand against an externally provisioned cluster.

## Autoscaling

Application Auto Scaling target tracking is on by default (`autoscaling_enabled = true`). The service tracks average CPU at `cpu_target` and average memory at `memory_target`, and drops the memory policy when that target is 0. `min_capacity` and `max_capacity` bound the task count, and `desired_count` is the starting count only: the service ignores later changes to it because Application Auto Scaling owns it.

`target_requests_per_second` adds an `ALBRequestCountPerTarget` policy next to the resource ones. The ALB publishes that metric as requests per minute per registered task, so the policy targets 60 times the variable: 90 requests per second becomes a target of 5400 per minute. Application Auto Scaling follows whichever policy asks for the most tasks, so the CPU and memory targets keep acting as a floor.

## Connection pool

Every task opens its own database connections, and with several tasks the total grows with the fleet. `connection_pool_enabled = true` runs a PgBouncer inside the task, shared by the server's connections, and sets `LITELLM_PGBOUNCER_ENABLED`, `LITELLM_PGBOUNCER_MAX_DB_CONNECTIONS` and `LITELLM_PGBOUNCER_MAX_CLIENT_CONN` on the container. `pool_max_db_connections` caps the upstream connections the task holds, whatever its worker count, and `pool_max_client_conn` bounds the client connections the pooler accepts.

```hcl
connection_pool_enabled = true
pool_max_db_connections = 20
pool_max_client_conn    = 1000
```

The pool needs a database, either the module's Aurora cluster or a supplied `database_url`, and the plan refuses the combination without one.

## Not built compared with LiteLLM

LiteLLM's module deploys a componentized stack of three services. This module deploys one, so these parts of LiteLLM's stack are absent by design:

- The gateway, backend and ui services and their path-based ALB routing. betterlite serves the data plane, the management routes and the admin UI from one process, so there is one service, one target group and no path rules.
- The collector sidecar and the Prometheus metrics sidecar, and the token-rate autoscaling policy that reads their metric.
- Enterprise billing metrics, its client certificate secrets and the env that points them at a collector.
- The separate migration image. One image ships both the server and `betterlite-db-admin`.

## Tests

The module tests plan the module against mock providers, so they need no AWS account, no credentials and no network beyond the provider download:

```bash
terraform -chdir=packaging/terraform/aws test
```

They cover the default layout, the bring-your-own network and store paths, identity and storage wiring, the service and load balancer, autoscaling, and the connection pool. The example root is checked by `terraform fmt`, `terraform init` and `terraform validate` in `examples/default/`; those resolve the module through `source = "../.."`, so a broken module fails there too.
