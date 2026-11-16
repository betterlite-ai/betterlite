resource "google_compute_network" "this" {
  count = local.create_network ? 1 : 0

  name                    = local.name
  auto_create_subnetworks = false
  routing_mode            = "REGIONAL"
}

resource "google_compute_subnetwork" "this" {
  count = local.create_network ? 1 : 0

  name                     = "${local.name}-${var.region}"
  region                   = var.region
  network                  = google_compute_network.this[0].id
  ip_cidr_range            = var.subnet_cidr
  private_ip_google_access = true
}

# Cloud SQL and Memorystore reach the VPC through this peering range, never through a public address
resource "google_compute_global_address" "psa" {
  count = var.create_psa_connection ? 1 : 0

  name          = "${local.name}-psa"
  purpose       = "VPC_PEERING"
  address_type  = "INTERNAL"
  prefix_length = 16
  network       = local.network_self_link
}

resource "google_service_networking_connection" "psa" {
  count = var.create_psa_connection ? 1 : 0

  network                 = local.network_self_link
  service                 = "servicenetworking.googleapis.com"
  reserved_peering_ranges = [google_compute_global_address.psa[0].name]
}

resource "google_vpc_access_connector" "this" {
  count = var.create_runtime ? 1 : 0

  # GCP caps a connector name at 25 characters, so the shared name is cut to 20 before the suffix
  name          = "${trimsuffix(substr(local.name, 0, 20), "-")}-conn"
  region        = var.region
  network       = local.network_self_link
  ip_cidr_range = var.vpc_connector_cidr
  machine_type  = "e2-micro"
  min_instances = 2
  max_instances = 3
}
