locals {
  # Google caps service account ids at 30 characters, so the shared prefix is cut once here
  name = trimsuffix(substr("${var.tenant}-${var.env}-betterlite", 0, 30), "-")

  labels = merge(
    {
      managed_by = "terraform"
    },
    var.labels,
  )

  create_network    = var.network_id == ""
  network_self_link = local.create_network ? google_compute_network.this[0].self_link : var.network_id
  network_name      = local.create_network ? google_compute_network.this[0].name : element(reverse(split("/", var.network_id)), 0)

  redis_enabled = var.create_redis
}
