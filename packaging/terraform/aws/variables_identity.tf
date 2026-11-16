variable "litellm_master_key" {
  description = "Pre-existing proxy master key, written to the master-key secret verbatim. Leave empty to have the module generate an sk- key."
  type        = string
  default     = ""
  sensitive   = true
}

variable "litellm_license" {
  description = "Enterprise license key, exposed to the proxy as LITELLM_LICENSE. When set the module creates a license secret; leave empty for an OSS-only deployment."
  type        = string
  default     = ""
  sensitive   = true
}

variable "ui_password" {
  description = "Admin UI password, exposed to the proxy as UI_PASSWORD. When set the module creates a ui-password secret; leave empty to log in with the master key."
  type        = string
  default     = ""
  sensitive   = true
}

variable "extra_secrets" {
  description = "Extra environment variables sourced from AWS Secrets Manager: map of environment variable name to secret ARN. Append :<jsonKey>:: to extract a single JSON field, as ECS allows. The execution role gains read access to each secret."
  type        = map(string)
  default     = {}
}

variable "otel_headers_secret_arn" {
  description = "Secrets Manager ARN whose plaintext value becomes OTEL_HEADERS. The execution role gains read access to it. Empty omits OTEL_HEADERS."
  type        = string
  default     = ""
}

variable "proxy_config" {
  description = "Proxy config (the contents of config.yaml), uploaded to the stack bucket as YAML and read by the service at startup. Empty uploads nothing."
  type        = any
  default     = {}
}

variable "s3_force_destroy" {
  description = "Allow terraform destroy to delete the stack bucket while it still holds objects. Default false keeps cached responses, archived request logs and uploaded files from being dropped by accident."
  type        = bool
  default     = false
}

variable "log_retention_days" {
  description = "CloudWatch log retention in days for the stack's log groups."
  type        = number
  default     = 30
}
