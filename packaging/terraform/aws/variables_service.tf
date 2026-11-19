variable "image" {
  description = "Container image running the betterlite server. There is no default: betterlite has no published image name yet, so the caller supplies one."
  type        = string

  validation {
    condition     = var.image != ""
    error_message = "image must name the betterlite container image to run."
  }
}

variable "cpu" {
  description = "Fargate CPU units for the betterlite task (1024 = 1 vCPU)."
  type        = number
  default     = 1024
}

variable "memory" {
  description = "Fargate memory (MiB) for the betterlite task."
  type        = number
  default     = 4096
}

variable "desired_count" {
  description = "Number of betterlite tasks to start with. Application Auto Scaling owns the count afterwards when autoscaling_enabled is true."
  type        = number
  default     = 2
}

variable "autoscaling_enabled" {
  description = "Turn on Application Auto Scaling target tracking for the service."
  type        = bool
  default     = true
}

variable "min_capacity" {
  description = "Minimum task count under autoscaling."
  type        = number
  default     = 1
}

variable "max_capacity" {
  description = "Maximum task count under autoscaling."
  type        = number
  default     = 10
}

variable "cpu_target" {
  description = "Target average CPU utilization (%) for the autoscaling policy."
  type        = number
  default     = 70
}

variable "memory_target" {
  description = "Target average memory utilization (%) for the autoscaling policy. Set 0 to scale on CPU only."
  type        = number
  default     = 80
}

variable "target_requests_per_second" {
  description = "Requests per second one task should serve, added as an ALBRequestCountPerTarget policy next to the CPU and memory ones. CloudWatch counts that metric per minute, so the policy targets 60 times this value. 0 skips the policy."
  type        = number
  default     = 0
}

variable "connection_pool_enabled" {
  description = "Run a PgBouncer connection pool inside the task, shared by the server's connections. Sets LITELLM_PGBOUNCER_ENABLED and the two pool limits; needs a database."
  type        = bool
  default     = false
}

variable "pool_max_db_connections" {
  description = "Upstream Postgres connections the task may hold when connection_pool_enabled is true."
  type        = number
  default     = 20

  validation {
    condition     = var.pool_max_db_connections >= 1
    error_message = "pool_max_db_connections must be at least 1."
  }
}

variable "pool_max_client_conn" {
  description = "Client connections the in-task PgBouncer accepts when connection_pool_enabled is true."
  type        = number
  default     = 1000

  validation {
    condition     = var.pool_max_client_conn >= 1
    error_message = "pool_max_client_conn must be at least 1."
  }
}

variable "extra_env" {
  description = "Additional plain-text environment variables for the container, for non-sensitive settings. A name that the module already sets replaces the module's value instead of adding a second entry."
  type        = map(string)
  default     = {}
}

variable "acm_certificate_arn" {
  description = "ACM certificate ARN for the ALB's HTTPS listener. When set, port 443 serves traffic and port 80 redirects to it; leave empty to serve HTTP only, which also needs allow_plaintext_alb."
  type        = string
  default     = ""
}

variable "allow_plaintext_alb" {
  description = "Serve the ALB over HTTP without TLS. Default false so a plan without acm_certificate_arn fails instead of shipping plaintext; meant for short-lived trial and dev stacks."
  type        = bool
  default     = false
}

variable "otel_endpoint" {
  description = "OTLP collector endpoint, which turns on the OpenTelemetry v2 environment block. Empty disables OTel entirely."
  type        = string
  default     = ""
}

variable "otel_exporter" {
  description = "OTLP exporter protocol for the OTel block. One of otlp_http, otlp_grpc or console."
  type        = string
  default     = "otlp_http"

  validation {
    condition     = contains(["otlp_http", "otlp_grpc", "console"], var.otel_exporter)
    error_message = "otel_exporter must be one of: otlp_http, otlp_grpc, console."
  }
}

variable "otel_environment_name" {
  description = "Value for OTEL_ENVIRONMENT_NAME, which tags every span with its deployment environment. Empty falls back to var.env."
  type        = string
  default     = ""
}

variable "otel_capture_message_content" {
  description = "Value for OTEL_INSTRUMENTATION_GENAI_CAPTURE_MESSAGE_CONTENT. The default no_content keeps prompts and completions out of the observability backend."
  type        = string
  default     = "no_content"

  validation {
    condition     = contains(["no_content", "prompt_and_completion"], var.otel_capture_message_content)
    error_message = "otel_capture_message_content must be one of: no_content, prompt_and_completion."
  }
}
