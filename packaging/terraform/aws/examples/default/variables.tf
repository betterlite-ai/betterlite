variable "region" {
  description = "AWS region to deploy into."
  type        = string
  default     = "us-east-1"
}

variable "tenant" {
  description = "Tenant slug used as the first part of every resource name."
  type        = string
  default     = "demo"
}

variable "env" {
  description = "Environment slug appended to every resource name."
  type        = string
  default     = "dev"
}

variable "image" {
  description = "Container image running the betterlite server. There is no default: betterlite has no published image name yet, so build and push one first."
  type        = string
}

variable "litellm_master_key" {
  description = "Proxy master key. The module writes it to the master-key secret verbatim; an empty value lets the module generate one."
  type        = string
  sensitive   = true
}

variable "acm_certificate_arn" {
  description = "ACM certificate ARN for the ALB's HTTPS listener. Leave empty to serve HTTP only, which then needs allow_plaintext_alb."
  type        = string
  default     = ""
}

variable "allow_plaintext_alb" {
  description = "Serve the ALB over HTTP without TLS, for a short-lived trial or dev stack."
  type        = bool
  default     = false
}
