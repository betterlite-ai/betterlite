#![cfg(test)]

use rstest::rstest;

#[rstest]
#[case::agentops_preset_defaults("agentops-preset-defaults.yaml")]
#[case::agentops_preset_key_and_environment("agentops-preset-key-and-environment.yaml")]
#[case::agentops_preset_empty_service_name("agentops-preset-empty-service-name.yaml")]
#[case::newrelic_preset_us_default("newrelic-preset-us-default.yaml")]
#[case::newrelic_preset_eu_region_upper_case("newrelic-preset-eu-region-upper-case.yaml")]
#[case::newrelic_preset_unknown_region("newrelic-preset-unknown-region.yaml")]
#[case::newrelic_preset_no_license_key("newrelic-preset-no-license-key.yaml")]
#[case::newrelic_preset_record_content_true("newrelic-preset-record-content-true.yaml")]
#[case::newrelic_preset_record_content_off("newrelic-preset-record-content-off.yaml")]
#[case::newrelic_preset_record_content_invalid("newrelic-preset-record-content-invalid.yaml")]
#[case::newrelic_preset_record_content_empty("newrelic-preset-record-content-empty.yaml")]
#[case::newrelic_dynamic_endpoint_eu("newrelic-dynamic-endpoint-eu.yaml")]
#[case::newrelic_dynamic_endpoint_unknown_region("newrelic-dynamic-endpoint-unknown-region.yaml")]
#[case::newrelic_dynamic_headers_api_key("newrelic-dynamic-headers-api-key.yaml")]
#[case::signoz_preset_key_and_endpoint("signoz-preset-key-and-endpoint.yaml")]
#[case::signoz_preset_without_key("signoz-preset-without-key.yaml")]
#[case::signoz_preset_empty_endpoint("signoz-preset-empty-endpoint.yaml")]
#[case::signoz_dynamic_endpoint_needs_key("signoz-dynamic-endpoint-needs-key.yaml")]
#[case::signoz_dynamic_endpoint_allowlisted("signoz-dynamic-endpoint-allowlisted.yaml")]
#[case::signoz_dynamic_endpoint_not_allowlisted("signoz-dynamic-endpoint-not-allowlisted.yaml")]
#[case::signoz_dynamic_headers_non_http_endpoint("signoz-dynamic-headers-non-http-endpoint.yaml")]
#[case::langfuse_preset_default_host("langfuse-preset-default-host.yaml")]
#[case::langfuse_preset_custom_host("langfuse-preset-custom-host.yaml")]
#[case::langfuse_preset_missing_keys_error("langfuse-preset-missing-keys-error.yaml")]
#[case::langfuse_preset_missing_keys_allowed("langfuse-preset-missing-keys-allowed.yaml")]
#[case::langfuse_dynamic_headers_both_keys("langfuse-dynamic-headers-both-keys.yaml")]
#[case::langfuse_dynamic_headers_missing_secret("langfuse-dynamic-headers-missing-secret.yaml")]
#[case::registry_unknown_callback("registry-unknown-callback.yaml")]
#[case::registry_dynamic_headers_empty_params("registry-dynamic-headers-empty-params.yaml")]
#[case::registry_dynamic_headers_levo_unsupported("registry-dynamic-headers-levo-unsupported.yaml")]
#[case::registry_dynamic_endpoint_newrelic_default(
    "registry-dynamic-endpoint-newrelic-default.yaml"
)]
#[case::registry_project_headers_phoenix_only("registry-project-headers-phoenix-only.yaml")]
#[case::destination_capable_backends("destination-capable-backends.yaml")]
#[case::destination_langfuse_us_default("destination-langfuse-us-default.yaml")]
#[case::destination_langfuse_tenant_host_allowlisted(
    "destination-langfuse-tenant-host-allowlisted.yaml"
)]
#[case::destination_langfuse_tenant_host_blocked("destination-langfuse-tenant-host-blocked.yaml")]
#[case::destination_langfuse_operator_host_unchecked(
    "destination-langfuse-operator-host-unchecked.yaml"
)]
#[case::destination_langfuse_span_scope_llm_only("destination-langfuse-span-scope-llm-only.yaml")]
#[case::destination_arize_grpc_endpoint("destination-arize-grpc-endpoint.yaml")]
#[case::destination_arize_missing_space_id("destination-arize-missing-space-id.yaml")]
#[case::destination_weave_host("destination-weave-host.yaml")]
#[case::destination_newrelic_eu("destination-newrelic-eu.yaml")]
#[case::destination_service_name_attribute("destination-service-name-attribute.yaml")]
#[case::destination_signoz_not_capable("destination-signoz-not-capable.yaml")]
fn helper_case_matches_litellm(#[case] name: &str) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("cases/helpers/otel-routing")
        .join(name);
    let cases = crate::case::load_cases(&[path]).expect("the case loads");
    let Some(crate::case::LoadedCase::Helper(case)) = cases.first() else {
        panic!("expected a helper case");
    };
    assert!(matches!(
        crate::helpers::run_case(case, false).expect("the case runs"),
        crate::helpers::HelperRun::Match
    ));
}
