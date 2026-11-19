# The module takes this provider's project and region through the call in
# main.tf. The block stays here and never moves into the module, because a
# module that configures its own provider cannot be handed a different project,
# an impersonated service account, or the count and for_each meta-arguments.
provider "google" {
  project = var.project_id
  region  = var.region
}
