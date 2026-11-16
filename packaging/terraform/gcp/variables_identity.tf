variable "litellm_master_key" {
  description = "Master key betterlite accepts as LITELLM_MASTER_KEY (an sk-... string). Required, because betterlite does not generate one for you."
  type        = string
  sensitive   = true
}

variable "litellm_license" {
  description = "LiteLLM license string. When set, the module creates a license secret and grants the runtime account access to it. Empty leaves it out for an OSS deployment."
  type        = string
  default     = ""
  sensitive   = true
}

variable "ui_password" {
  description = "Admin UI password. When set, the module creates a ui-password secret and grants the runtime account access to it. Empty logs the UI in with the master key."
  type        = string
  default     = ""
  sensitive   = true
}

variable "extra_secrets" {
  description = "Environment variable name to the Secret Manager secret id (`projects/<project>/secrets/<name>`) of a secret the caller created. The runtime account gets secretAccessor on each distinct secret."
  type        = map(string)
  default     = {}
}

variable "proxy_config" {
  description = "betterlite proxy config, the same YAML-shaped map the Helm chart and the AWS module take. When it has keys the module uploads it as config.yaml to a dedicated bucket."
  type        = any
  default     = {}
}

variable "bucket_force_destroy" {
  description = "Let `terraform destroy` delete the data and config buckets while they still hold objects. Set true only for ephemeral environments."
  type        = bool
  default     = false
}

variable "bucket_location" {
  description = "GCS location for both buckets, a region or a multi-region such as EU. Empty means var.region."
  type        = string
  default     = ""
}

variable "data_retention_days" {
  description = "Days after which the data bucket deletes its objects through a lifecycle rule. 0 means no rule, so objects stay until someone removes them."
  type        = number
  default     = 0

  validation {
    condition     = var.data_retention_days >= 0
    error_message = "data_retention_days must be 0 or more."
  }
}
