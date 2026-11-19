mock_provider "google" {
  override_during = plan

  mock_resource "google_compute_network" {
    defaults = {
      id        = "projects/test-project/global/networks/tenant-test-betterlite"
      self_link = "https://www.googleapis.com/compute/v1/projects/test-project/global/networks/tenant-test-betterlite"
    }
  }

  mock_resource "google_redis_instance" {
    defaults = {
      host = "10.0.0.4"
      port = 6379
    }
  }

  mock_resource "google_compute_global_address" {
    defaults = {
      address = "203.0.113.10"
    }
  }
}

mock_provider "random" {
  override_during = plan

  mock_resource "random_password" {}
  mock_resource "random_id" {
    defaults = {
      hex = "deadbeef"
    }
  }
}

variables {
  project_id         = "test-project"
  tenant             = "tenant"
  env                = "test"
  litellm_master_key = "sk-test-master-key"
  image              = "us-central1-docker.pkg.dev/test-project/betterlite/betterlite:test"
  allow_plaintext_lb = true
}

run "one_neg_and_one_backend_service" {
  command = plan

  assert {
    condition = alltrue([
      length(google_compute_region_network_endpoint_group.betterlite) == 1,
      google_compute_region_network_endpoint_group.betterlite[0].name == "tenant-test-betterlite-neg",
      google_compute_region_network_endpoint_group.betterlite[0].network_endpoint_type == "SERVERLESS",
      google_compute_region_network_endpoint_group.betterlite[0].region == "us-central1",
      one(google_compute_region_network_endpoint_group.betterlite[0].cloud_run).service == google_cloud_run_v2_service.betterlite[0].name,
      length(google_compute_backend_service.betterlite) == 1,
      google_compute_backend_service.betterlite[0].name == "tenant-test-betterlite-bs",
      google_compute_backend_service.betterlite[0].protocol == "HTTPS",
      google_compute_backend_service.betterlite[0].load_balancing_scheme == "EXTERNAL_MANAGED",
      google_compute_backend_service.betterlite[0].timeout_sec == 300,
      length(google_compute_backend_service.betterlite[0].backend) == 1,
      one(google_compute_backend_service.betterlite[0].backend).group == google_compute_region_network_endpoint_group.betterlite[0].id,
    ])
    error_message = "The load balancer must have one serverless NEG for the service and one EXTERNAL_MANAGED HTTPS backend service that uses only that NEG, with the request timeout from timeout_seconds."
  }
}

run "url_map_serves_only_a_default_service" {
  command = plan

  assert {
    condition = alltrue([
      length(google_compute_url_map.betterlite) == 1,
      google_compute_url_map.betterlite[0].name == "tenant-test-betterlite",
      google_compute_url_map.betterlite[0].default_service == google_compute_backend_service.betterlite[0].id,
      try(length(google_compute_url_map.betterlite[0].path_matcher), 0) == 0,
      try(length(google_compute_url_map.betterlite[0].host_rule), 0) == 0,
    ])
    error_message = "The URL map must send every request to the one backend service, with no host rule and no path rule."
  }
}

run "http_only_without_domains" {
  command = plan

  assert {
    condition = alltrue([
      google_compute_global_address.lb[0].name == "tenant-test-betterlite-lb-ip",
      length(google_compute_target_http_proxy.http) == 1,
      google_compute_target_http_proxy.http[0].name == "tenant-test-betterlite-http",
      google_compute_target_http_proxy.http[0].url_map == google_compute_url_map.betterlite[0].id,
      length(google_compute_global_forwarding_rule.http) == 1,
      google_compute_global_forwarding_rule.http[0].name == "tenant-test-betterlite-http",
      google_compute_global_forwarding_rule.http[0].ip_protocol == "TCP",
      google_compute_global_forwarding_rule.http[0].load_balancing_scheme == "EXTERNAL_MANAGED",
      google_compute_global_forwarding_rule.http[0].port_range == "80",
      google_compute_global_forwarding_rule.http[0].target == google_compute_target_http_proxy.http[0].id,
      google_compute_global_forwarding_rule.http[0].ip_address == google_compute_global_address.lb[0].address,
      length(google_compute_url_map.https_redirect) == 0,
      length(google_compute_managed_ssl_certificate.betterlite) == 0,
      length(google_compute_target_https_proxy.https) == 0,
      length(google_compute_global_forwarding_rule.https) == 0,
      output.lb_ip == google_compute_global_address.lb[0].address,
      output.lb_url == "http://${google_compute_global_address.lb[0].address}",
    ])
    error_message = "Without lb_domains and with allow_plaintext_lb the port-80 rule must serve the main URL map and no HTTPS resource may exist."
  }
}

run "domains_add_the_https_frontend" {
  command = plan

  variables {
    lb_domains         = ["api.example.com", "example.com"]
    allow_plaintext_lb = false
  }

  assert {
    condition = alltrue([
      length(google_compute_managed_ssl_certificate.betterlite) == 1,
      google_compute_managed_ssl_certificate.betterlite[0].name == "tenant-test-betterlite-cert-4ffadfe4",
      join(",", one(google_compute_managed_ssl_certificate.betterlite[0].managed).domains) == "api.example.com,example.com",
      length(google_compute_url_map.https_redirect) == 1,
      google_compute_url_map.https_redirect[0].name == "tenant-test-betterlite-redirect",
      one(google_compute_url_map.https_redirect[0].default_url_redirect).https_redirect == true,
      one(google_compute_url_map.https_redirect[0].default_url_redirect).strip_query == false,
      one(google_compute_url_map.https_redirect[0].default_url_redirect).redirect_response_code == "MOVED_PERMANENTLY_DEFAULT",
      google_compute_target_http_proxy.http[0].url_map == google_compute_url_map.https_redirect[0].id,
      length(google_compute_target_https_proxy.https) == 1,
      google_compute_target_https_proxy.https[0].name == "tenant-test-betterlite-https",
      join(",", google_compute_target_https_proxy.https[0].ssl_certificates) == google_compute_managed_ssl_certificate.betterlite[0].id,
      google_compute_target_https_proxy.https[0].url_map == google_compute_url_map.betterlite[0].id,
      length(google_compute_global_forwarding_rule.https) == 1,
      google_compute_global_forwarding_rule.https[0].name == "tenant-test-betterlite-https",
      google_compute_global_forwarding_rule.https[0].load_balancing_scheme == "EXTERNAL_MANAGED",
      google_compute_global_forwarding_rule.https[0].port_range == "443",
      google_compute_global_forwarding_rule.https[0].target == google_compute_target_https_proxy.https[0].id,
      google_compute_global_forwarding_rule.https[0].ip_address == google_compute_global_address.lb[0].address,
      output.lb_ip == google_compute_global_address.lb[0].address,
      output.lb_url == "https://api.example.com",
    ])
    error_message = "lb_domains must add the managed certificate, the redirect URL map on the port-80 proxy and the HTTPS proxy with a 443 forwarding rule."
  }
}
