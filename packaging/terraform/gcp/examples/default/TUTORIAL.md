# Deploy betterlite on GCP

<walkthrough-tutorial-duration duration="20"></walkthrough-tutorial-duration>

This walkthrough provisions the betterlite stack on GCP: a Cloud Run service behind an external load balancer, Cloud SQL for PostgreSQL with a read replica, Memorystore Redis, two GCS buckets, Secret Manager entries and a Cloud Run migration job that runs before the service starts.

## Pick the project

<walkthrough-project-billing-setup></walkthrough-project-billing-setup>

Choose the project to deploy into and leave it selected for the rest of the walkthrough. The stack creates paid resources: Cloud SQL, Memorystore and a load balancer address.

## Enable the APIs

Click the list to enable the APIs, or run the gcloud command.

<walkthrough-enable-apis apis="run.googleapis.com,sqladmin.googleapis.com,redis.googleapis.com,secretmanager.googleapis.com,vpcaccess.googleapis.com,compute.googleapis.com,servicenetworking.googleapis.com,storage.googleapis.com"></walkthrough-enable-apis>

```bash
gcloud services enable \
  run.googleapis.com \
  sqladmin.googleapis.com \
  redis.googleapis.com \
  secretmanager.googleapis.com \
  vpcaccess.googleapis.com \
  compute.googleapis.com \
  servicenetworking.googleapis.com \
  storage.googleapis.com
```

## Write terraform.tfvars

Copy the sample and fill in the project, the image and a master key of your own.

```bash
cp terraform.tfvars.example terraform.tfvars
```

- `project_id`: the project you picked above.
- `image`: a container image Cloud Run can pull, holding betterlite and betterlite-db-admin.
- `litellm_master_key`: replace `sk-change-me` with a secret of your own.
- `allow_plaintext_lb`: set `true` if you have no DNS name yet. Without `lb_domains` the load balancer has no certificate, and the apply stops unless plaintext is allowed.

## Initialize and apply

```bash
terraform init
terraform apply
```

The first apply takes about 20 minutes; Cloud SQL takes most of it. The migration job runs once the database is ready, and the service starts after it succeeds.

## Call the service

```bash
terraform output -raw lb_url
curl "$(terraform output -raw lb_url)/health/readiness"
```

`/health/liveliness` answers as soon as the process is up; `/health/readiness` turns healthy once the database and Redis are reachable.

## Tear it down

```bash
terraform destroy
```

Cloud SQL has deletion protection on by default; the destroy stops on the database. Set `cloudsql_deletion_protection = false` on the `module "betterlite"` block in `main.tf` and apply once more to let it go.

Full configuration reference is in `<walkthrough-editor-open-file filePath="packaging/terraform/gcp/README.md">README.md</walkthrough-editor-open-file>`, and every input the module takes is in `<walkthrough-editor-open-file filePath="packaging/terraform/gcp/variables_service.tf">variables_service.tf</walkthrough-editor-open-file>` and its two sibling files.
