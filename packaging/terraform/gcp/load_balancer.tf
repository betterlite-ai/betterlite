locals {
  tls_enabled = length(var.lb_domains) > 0
}

resource "google_compute_global_address" "lb" {
  count = var.create_runtime ? 1 : 0

  name   = "${local.name}-lb-ip"
  labels = local.labels
}

# betterlite serves the API, the management routes and the UI from one service, so one NEG fronts it
resource "google_compute_region_network_endpoint_group" "betterlite" {
  count = var.create_runtime ? 1 : 0

  name                  = "${local.name}-neg"
  region                = var.region
  network_endpoint_type = "SERVERLESS"

  cloud_run {
    service = google_cloud_run_v2_service.betterlite[0].name
  }
}

resource "google_compute_backend_service" "betterlite" {
  count = var.create_runtime ? 1 : 0

  name                  = "${local.name}-bs"
  protocol              = "HTTPS"
  load_balancing_scheme = "EXTERNAL_MANAGED"
  timeout_sec           = var.timeout_seconds

  backend {
    group = google_compute_region_network_endpoint_group.betterlite[0].id
  }

  # TLS is the supported posture; plain HTTP needs an explicit opt-in for trials and development
  lifecycle {
    precondition {
      condition     = local.tls_enabled || var.allow_plaintext_lb
      error_message = "The load balancer has no HTTPS forwarding rule. Set lb_domains to DNS names the module gets a Google-managed certificate for, or set allow_plaintext_lb = true to serve HTTP only."
    }
  }
}

resource "google_compute_url_map" "betterlite" {
  count = var.create_runtime ? 1 : 0

  name            = local.name
  default_service = google_compute_backend_service.betterlite[0].id
}

# Permanent HTTP to HTTPS redirect, attached to the port-80 proxy only when lb_domains is set
resource "google_compute_url_map" "https_redirect" {
  count = var.create_runtime && local.tls_enabled ? 1 : 0

  name = "${local.name}-redirect"

  default_url_redirect {
    https_redirect         = true
    redirect_response_code = "MOVED_PERMANENTLY_DEFAULT"
    strip_query            = false
  }
}

resource "google_compute_target_http_proxy" "http" {
  count = var.create_runtime ? 1 : 0

  name    = "${local.name}-http"
  url_map = local.tls_enabled ? google_compute_url_map.https_redirect[0].id : google_compute_url_map.betterlite[0].id
}

resource "google_compute_global_forwarding_rule" "http" {
  count = var.create_runtime ? 1 : 0

  name                  = "${local.name}-http"
  ip_protocol           = "TCP"
  port_range            = "80"
  load_balancing_scheme = "EXTERNAL_MANAGED"
  ip_address            = google_compute_global_address.lb[0].address
  target                = google_compute_target_http_proxy.http[0].id
  labels                = local.labels
}

# A managed certificate's domain list cannot change in place, so the name follows it and create_before_destroy swaps it
resource "google_compute_managed_ssl_certificate" "betterlite" {
  count = var.create_runtime && local.tls_enabled ? 1 : 0

  name = "${local.name}-cert-${substr(sha1(join(",", var.lb_domains)), 0, 8)}"

  managed {
    domains = var.lb_domains
  }

  lifecycle {
    create_before_destroy = true
  }
}

resource "google_compute_target_https_proxy" "https" {
  count = var.create_runtime && local.tls_enabled ? 1 : 0

  name             = "${local.name}-https"
  url_map          = google_compute_url_map.betterlite[0].id
  ssl_certificates = [google_compute_managed_ssl_certificate.betterlite[0].id]
}

resource "google_compute_global_forwarding_rule" "https" {
  count = var.create_runtime && local.tls_enabled ? 1 : 0

  name                  = "${local.name}-https"
  ip_protocol           = "TCP"
  port_range            = "443"
  load_balancing_scheme = "EXTERNAL_MANAGED"
  ip_address            = google_compute_global_address.lb[0].address
  target                = google_compute_target_https_proxy.https[0].id
  labels                = local.labels
}
