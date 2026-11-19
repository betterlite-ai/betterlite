# The handful of inputs the one-command deploy asks a caller for. Everything
# else the module accepts (sizing, database and Redis tiers, the proxy config,
# the buckets) keeps its default; set it on the module "betterlite" block in
# main.tf, or call the module from a root of your own. The module's variables
# files list every input with its type, default and description.

variable "project_id" {
  description = "GCP project that holds the network, the data stores, the runtime and their secrets."
  type        = string
}

variable "region" {
  description = "GCP region for the VPC, Cloud SQL, Memorystore, Cloud Run and the load balancer IP."
  type        = string
  default     = "us-central1"
}

variable "tenant" {
  description = "Tenant slug, the first part of every resource name. Combined with env it forms <tenant>-<env>-betterlite."
  type        = string
  default     = "demo"
}

variable "env" {
  description = "Environment slug, the second part of every resource name."
  type        = string
  default     = "dev"
}

variable "image" {
  description = "Container image holding betterlite and betterlite-db-admin."
  type        = string
}

variable "litellm_master_key" {
  description = "Master key betterlite accepts as LITELLM_MASTER_KEY (an sk-... string)."
  type        = string
  sensitive   = true
}

variable "lb_domains" {
  description = "DNS names that point at lb_ip. When set, the module adds a Google-managed certificate and an HTTPS forwarding rule and redirects HTTP to HTTPS."
  type        = list(string)
  default     = []
}

variable "allow_plaintext_lb" {
  description = "Serve plain HTTP on the load balancer when lb_domains is empty. Use only for a trial or a development environment."
  type        = bool
  default     = false
}
