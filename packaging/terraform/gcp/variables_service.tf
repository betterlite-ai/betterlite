variable "image" {
  description = "Container image for the betterlite Cloud Run service and the migration job. B5071 ships betterlite and betterlite-db-admin in one image."
  type        = string

  validation {
    condition     = var.image != ""
    error_message = "image must name the betterlite container image."
  }
}

variable "cpu" {
  description = "CPU limit for the Cloud Run container. Cloud Run accepts 1, 2, 4 and 8."
  type        = string
  default     = "2"
}

variable "memory" {
  description = "Memory limit for the Cloud Run container."
  type        = string
  default     = "4Gi"
}

variable "min_instances" {
  description = "Minimum number of Cloud Run instances kept running."
  type        = number
  default     = 1
}

variable "max_instances" {
  description = "Maximum number of Cloud Run instances."
  type        = number
  default     = 10
}

variable "concurrency" {
  description = "Maximum concurrent requests each Cloud Run instance accepts."
  type        = number
  default     = 80
}

variable "timeout_seconds" {
  description = "Request timeout in seconds for the Cloud Run service and the load balancer backend service."
  type        = number
  default     = 300
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

variable "extra_env" {
  description = "Additional environment variables for the service. A name the module also sets replaces the module's value."
  type        = map(string)
  default     = {}
}

variable "connection_pool_enabled" {
  description = "Route the service through a PgBouncer connection pool in front of Cloud SQL."
  type        = bool
  default     = false
}

variable "pool_max_db_connections" {
  description = "Maximum database connections the PgBouncer pool opens."
  type        = number
  default     = 20
}

variable "pool_max_client_conn" {
  description = "Maximum client connections the PgBouncer pool accepts."
  type        = number
  default     = 1000
}

variable "otel_endpoint" {
  description = "OTLP endpoint for OpenTelemetry traces and metrics. Empty leaves OpenTelemetry off."
  type        = string
  default     = ""
}
