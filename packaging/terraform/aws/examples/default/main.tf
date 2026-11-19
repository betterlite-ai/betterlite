module "betterlite" {
  source = "../.."

  region             = var.region
  tenant             = var.tenant
  env                = var.env
  image              = var.image
  litellm_master_key = var.litellm_master_key

  # The module needs two AZs for its VPC, Aurora and ElastiCache subnet groups; the example takes the first two of the region
  azs = ["${var.region}a", "${var.region}b"]

  acm_certificate_arn = var.acm_certificate_arn
  allow_plaintext_alb = var.allow_plaintext_alb
}
