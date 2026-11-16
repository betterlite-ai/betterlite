resource "google_redis_instance" "this" {
  count = var.create_redis ? 1 : 0

  name           = local.name
  tier           = var.redis_tier
  memory_size_gb = var.redis_memory_size_gb
  region         = var.region

  authorized_network = local.network_self_link
  connect_mode       = "PRIVATE_SERVICE_ACCESS"

  redis_version = "REDIS_7_0"

  labels = local.labels

  # AUTH stays on so an instance in the shared PSA range is not open to every client that can reach it
  auth_enabled            = true
  transit_encryption_mode = var.redis_transit_encryption ? "SERVER_AUTHENTICATION" : "DISABLED"

  depends_on = [google_service_networking_connection.psa]
}
