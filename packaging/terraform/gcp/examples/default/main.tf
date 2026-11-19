module "betterlite" {
  source = "../.."

  project_id = var.project_id
  region     = var.region
  tenant     = var.tenant
  env        = var.env

  image              = var.image
  litellm_master_key = var.litellm_master_key

  lb_domains         = var.lb_domains
  allow_plaintext_lb = var.allow_plaintext_lb
}

# The runnable example: this root owns the google provider and deploys the whole
# stack in one apply.
#
#   cd packaging/terraform/gcp/examples/default
#   cp terraform.tfvars.example terraform.tfvars
#   edit terraform.tfvars, then:
#   terraform init
#   terraform apply
#
# Another root can call the module with count, for_each or an impersonated
# service account, because the module declares no provider:
#
#   module "betterlite" {
#     source = "github.com/<org>/betterlite//packaging/terraform/gcp?ref=<tag>"
#     ...
#   }
#
# It declares no configuration_aliases either, so every call receives the
# caller's single default google provider: a for_each runs each instance
# against the same project, region and credentials. Use one root per project to
# fan out across projects or regions.
#
# Sizing knobs (cpu, memory, min_instances, max_instances, db_tier, redis_tier,
# proxy_config and the rest) are not variables of this root. Set them directly
# on this block; ../.. lists every input.
