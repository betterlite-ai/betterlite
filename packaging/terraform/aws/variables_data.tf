variable "region" {
  description = "AWS region the module deploys into."
  type        = string
}

variable "tenant" {
  description = "Tenant slug used as the first part of every resource name; resources are named <tenant>-litellm-<env>."
  type        = string

  validation {
    condition     = can(regex("^[a-z][a-z0-9-]{0,20}$", var.tenant))
    error_message = "tenant must be 1-21 characters of lower-case letters, digits and hyphens, starting with a letter."
  }
}

variable "env" {
  description = "Environment slug appended to every resource name."
  type        = string

  validation {
    condition     = can(regex("^[a-z][a-z0-9-]{0,8}$", var.env))
    error_message = "env must be 1-9 characters of lower-case letters, digits and hyphens, starting with a letter."
  }
}

variable "tags" {
  description = "Extra tags applied to every taggable resource, merged over the module's own litellm:stack and managed-by tags."
  type        = map(string)
  default     = {}
}

variable "vpc_id" {
  description = "Existing VPC to deploy into. Leave empty to have the module create a VPC, subnets, NAT gateway and route tables. When set, public_subnet_ids and private_subnet_ids are required and no networking is created."
  type        = string
  default     = ""
}

variable "public_subnet_ids" {
  description = "Existing public subnets for the load balancer. Required when vpc_id is set, ignored otherwise."
  type        = list(string)
  default     = []
}

variable "private_subnet_ids" {
  description = "Existing private subnets for the tasks, Aurora and ElastiCache. Required when vpc_id is set, ignored otherwise."
  type        = list(string)
  default     = []
}

variable "additional_task_security_group_ids" {
  description = "Extra security groups attached to the tasks on top of the module's own group. Useful with vpc_id: attach a group an existing database or cache already allows inbound from."
  type        = list(string)
  default     = []
}

variable "vpc_cidr" {
  description = "CIDR block for the VPC the module creates. Unused when vpc_id is set."
  type        = string
  default     = "10.40.0.0/16"
}

variable "azs" {
  description = "Availability zones the module-created subnets spread across. At least 2 are needed for Aurora and the load balancer. Unused when vpc_id is set."
  type        = list(string)
  default     = []
}

variable "create_database" {
  description = "Create the Aurora Postgres cluster. Set false to skip it and either point the stack at an existing database with database_url or run without a database."
  type        = bool
  default     = true
}

variable "database_url" {
  description = "Postgres connection string for an existing database, for example postgresql://user:pass@host:5432/litellm. Only read when create_database is false."
  type        = string
  default     = ""
  sensitive   = true
}

variable "db_instance_class" {
  description = "Aurora instance class for the writer and reader instances."
  type        = string
  default     = "db.r6g.large"
}

variable "db_engine_version" {
  description = "Aurora Postgres engine version. Its major version selects the cluster parameter group family."
  type        = string
  default     = "16.4"
}

variable "db_name" {
  description = "Initial database created on the Aurora cluster."
  type        = string
  default     = "litellm"
}

variable "db_master_username" {
  description = "Aurora master username, used only to bootstrap the application user."
  type        = string
  default     = "postgres"
}

variable "db_username" {
  description = "Postgres user the tasks connect as, created in the cluster with the rds_iam role."
  type        = string
  default     = "litellm_app"
}

variable "create_redis" {
  description = "Create the ElastiCache Redis replication group. Set false to skip it and either point the stack at an existing cache with redis_url or run without Redis."
  type        = bool
  default     = true
}

variable "redis_url" {
  description = "Connection string for an existing Redis, for example rediss://:password@host:6379. Only read when create_redis is false."
  type        = string
  default     = ""
  sensitive   = true
}

variable "redis_node_type" {
  description = "ElastiCache node type."
  type        = string
  default     = "cache.t4g.small"
}

variable "redis_num_replicas" {
  description = "Read replicas in the Redis replication group. The primary plus this many replicas form the cluster; automatic failover and multi-AZ need at least 1."
  type        = number
  default     = 1

  validation {
    condition     = var.redis_num_replicas >= 0
    error_message = "redis_num_replicas must be 0 or more."
  }
}

variable "skip_final_snapshot" {
  description = "Skip the Aurora final snapshot on destroy. Default false, so destroying the cluster takes a snapshot first."
  type        = bool
  default     = false
}
