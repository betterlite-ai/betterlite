#![cfg(test)]

#[test]
fn helper_functions_dispatch() {
    let input = serde_json::json!({"function":"cache_token_value","values":[0_i64,4_i64]});
    let result =
        <super::Op as crate::helpers::Helper>::run(&serde_json::from_value(input).unwrap())
            .unwrap();
    assert_eq!(result, serde_json::json!({"result":4_i64}));
}

#[rstest::rstest]
#[case::cache_absent("cache-absent.yaml")]
#[case::cache_invalid_before_zero("cache-invalid-before-zero.yaml")]
#[case::cache_positive_priority("cache-positive-priority.yaml")]
#[case::cache_zero_before_invalid("cache-zero-before-invalid.yaml")]
#[case::cost_complete("cost-complete.yaml")]
#[case::cost_sparse_breakdown("cost-sparse-breakdown.yaml")]
#[case::dict_items("dict-items.yaml")]
#[case::guardrail_complete("guardrail-complete.yaml")]
#[case::guardrail_error_statuses_detection("guardrail-error-statuses-detection.yaml")]
#[case::guardrail_failed_status("guardrail-failed-status.yaml")]
#[case::guardrail_passed_status("guardrail-passed-status.yaml")]
#[case::json_null("json-null.yaml")]
#[case::json_string("json-string.yaml")]
#[case::masked_counts("masked-counts.yaml")]
#[case::masked_zero("masked-zero.yaml")]
#[case::mode_absent("mode-absent.yaml")]
#[case::mode_list("mode-list.yaml")]
#[case::params_absent("params-absent.yaml")]
#[case::params_null("params-null.yaml")]
#[case::params_zero("params-zero.yaml")]
#[case::proxy_request("proxy-request.yaml")]
#[case::request_params_all_fields_optional("request-params-all-fields-optional.yaml")]
#[case::sanitize_bounds_count_and_length("sanitize-bounds-count-and-length.yaml")]
#[case::server_custom_scheme("server-custom-scheme.yaml")]
#[case::server_explicit_default_port("server-explicit-default-port.yaml")]
#[case::server_https_implicit_port("server-https-implicit-port.yaml")]
#[case::server_info_api_base_port_defaults("server-info-api-base-port-defaults.yaml")]
#[case::server_missing_authority("server-missing-authority.yaml")]
#[case::service_error("service-error.yaml")]
#[case::service_sanitize_event_metadata_secrets("service-sanitize-event-metadata-secrets.yaml")]
#[case::span_error_absent("span-error-absent.yaml")]
#[case::span_error_complete("span-error-complete.yaml")]
#[case::usage_absent("usage-absent.yaml")]
#[case::usage_cache_token_value_priority("usage-cache-token-value-priority.yaml")]
#[case::server_unicode_host("server-unicode-host.yaml")]
#[case::server_expanded_ipv6("server-expanded-ipv6.yaml")]
#[case::params_invalid("params-invalid.yaml")]
#[case::params_small_float("params-small-float.yaml")]
fn payload_case_matches_litellm(#[case] name: &str) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("cases/helpers/payload-primitives")
        .join(name);
    let cases = crate::case::load_cases(&[path]).unwrap();
    let Some(crate::case::LoadedCase::Helper(case)) = cases.first() else {
        panic!("expected a helper case");
    };
    assert!(matches!(
        crate::helpers::run_case(case, false).unwrap(),
        crate::helpers::HelperRun::Match
    ));
}
