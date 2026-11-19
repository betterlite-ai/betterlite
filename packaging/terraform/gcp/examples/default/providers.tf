provider "google" {
  project = var.project_id
  region  = var.region
}

# The block stays in the root and never moves into the module: a module that
# configures its own provider cannot be handed a different project, an
# impersonated service account, or the count and for_each meta-arguments. The
# module's resources inherit this default provider through the call in main.tf.
