provider "aws" {
  region = var.region

  # Org-wide tags go here; they reach every resource the module creates and merge with its own tags
  default_tags {
    tags = {
      managed_by = "terraform"
    }
  }
}
