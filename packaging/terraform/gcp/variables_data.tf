variable "project_id" {
  description = "GCP project that holds the network, the data stores and their secrets."
  type        = string
}

variable "region" {
  description = "GCP region for the VPC, Cloud SQL, Memorystore and the serverless connector."
  type        = string
  default     = "us-central1"
}

variable "tenant" {
  description = "Tenant slug, the first part of every resource name. Combined with env it forms `<tenant>-<env>-betterlite`."
  type        = string
  default     = "demo"

  validation {
    condition     = can(regex("^[a-z][a-z0-9-]{0,20}$", var.tenant))
    error_message = "tenant must be 1-21 chars, lower-kebab-case, starting with a letter."
  }
}

variable "env" {
  description = "Environment slug, the second part of every resource name."
  type        = string
  default     = "dev"

  validation {
    condition     = can(regex("^[a-z][a-z0-9-]{0,8}$", var.env))
    error_message = "env must be 1-9 chars, lower-kebab-case, starting with a letter."
  }
}

variable "labels" {
  description = "Labels applied to every label-supporting resource, on top of the module's managed_by label."
  type        = map(string)
  default     = {}
}

variable "create_runtime" {
  description = "Create the runtime side of the module. Set false to build only the network and the data stores for a caller that runs the service elsewhere."
  type        = bool
  default     = true
}

variable "network_id" {
  description = "Existing VPC network resource id (`projects/<project>/global/networks/<name>`). When set, no VPC or subnet is created."
  type        = string
  default     = ""
}

variable "create_psa_connection" {
  description = "Create the private services access range and connection that Cloud SQL and Memorystore need. Set false when the existing network already has private services access."
  type        = bool
  default     = true
}

variable "subnet_cidr" {
  description = "Primary CIDR block for the betterlite subnet. Unused when network_id is set."
  type        = string
  default     = "10.20.0.0/24"
}

variable "vpc_connector_cidr" {
  description = "CIDR block for the serverless VPC Access connector. A /28 is required. Unused when create_runtime is false."
  type        = string
  default     = "10.20.1.0/28"
}

variable "db_tier" {
  description = "Cloud SQL machine type for the writer and the reader."
  type        = string
  default     = "db-custom-2-7680"
}

variable "db_edition" {
  description = "Cloud SQL edition. ENTERPRISE accepts the db-custom-* tiers; ENTERPRISE_PLUS only accepts db-perf-optimized-* tiers."
  type        = string
  default     = "ENTERPRISE"

  validation {
    condition     = contains(["ENTERPRISE", "ENTERPRISE_PLUS"], var.db_edition)
    error_message = "db_edition must be ENTERPRISE or ENTERPRISE_PLUS."
  }
}

variable "db_version" {
  description = "Cloud SQL Postgres version."
  type        = string
  default     = "POSTGRES_16"
}

variable "db_name" {
  description = "Database created on the Cloud SQL writer."
  type        = string
  default     = "betterlite"
}

variable "db_username" {
  description = "Application Postgres user. Its password is generated and stored in Secret Manager."
  type        = string
  default     = "betterlite"
}

variable "cloudsql_deletion_protection" {
  description = "Cloud SQL instance deletion protection on the writer and the reader. Set false only for ephemeral environments."
  type        = bool
  default     = true
}

variable "redis_tier" {
  description = "Memorystore tier: STANDARD_HA for production, BASIC for development."
  type        = string
  default     = "STANDARD_HA"
}

variable "redis_memory_size_gb" {
  description = "Memorystore capacity in GB."
  type        = number
  default     = 1
}

variable "redis_transit_encryption" {
  description = "Enable Memorystore in-transit encryption. The client then needs the server CA, which betterlite cannot read yet, so this defaults to false."
  type        = bool
  default     = false
}

variable "create_redis" {
  description = "Create the Memorystore instance. Set false to use a Redis the caller runs."
  type        = bool
  default     = true
}
