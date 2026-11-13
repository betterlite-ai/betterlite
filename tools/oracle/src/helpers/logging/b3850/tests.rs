#![cfg(test)]

use rstest::rstest;

#[rstest]
#[case::arize_config_default_grpc("arize-config-default-grpc.yaml")]
#[case::arize_config_grpc_endpoint_wins("arize-config-grpc-endpoint-wins.yaml")]
#[case::arize_config_http_endpoint("arize-config-http-endpoint.yaml")]
#[case::arize_preset_space_id_and_api_key("arize-preset-space-id-and-api-key.yaml")]
#[case::arize_preset_space_key_fallback("arize-preset-space-key-fallback.yaml")]
#[case::arize_preset_otlp_headers_fallback("arize-preset-otlp-headers-fallback.yaml")]
#[case::arize_preset_project_resource("arize-preset-project-resource.yaml")]
#[case::arize_dynamic_headers_space_key_wins("arize-dynamic-headers-space-key-wins.yaml")]
#[case::phoenix_config_default_local("phoenix-config-default-local.yaml")]
#[case::phoenix_config_grpc_scheme("phoenix-config-grpc-scheme.yaml")]
#[case::phoenix_config_port_4317("phoenix-config-port-4317.yaml")]
#[case::phoenix_config_path_ends_v1("phoenix-config-path-ends-v1.yaml")]
#[case::phoenix_config_path_ends_slash("phoenix-config-path-ends-slash.yaml")]
#[case::phoenix_config_path_bare_host("phoenix-config-path-bare-host.yaml")]
#[case::phoenix_config_cloud_endpoint_unchanged("phoenix-config-cloud-endpoint-unchanged.yaml")]
#[case::phoenix_config_cloud_without_key("phoenix-config-cloud-without-key.yaml")]
#[case::phoenix_config_http_variable_wins("phoenix-config-http-variable-wins.yaml")]
#[case::phoenix_config_header_casing_grpc("phoenix-config-header-casing-grpc.yaml")]
#[case::phoenix_preset_project_alias("phoenix-preset-project-alias.yaml")]
#[case::phoenix_project_headers_override_wins("phoenix-project-headers-override-wins.yaml")]
#[case::levo_config_headers("levo-config-headers.yaml")]
#[case::levo_config_missing_api_key("levo-config-missing-api-key.yaml")]
#[case::levo_config_missing_collector_url("levo-config-missing-collector-url.yaml")]
#[case::levo_preset_keeps_mappers("levo-preset-keeps-mappers.yaml")]
#[case::weave_config_cloud_default("weave-config-cloud-default.yaml")]
#[case::weave_config_host_without_scheme("weave-config-host-without-scheme.yaml")]
#[case::weave_config_host_with_scheme_and_slash("weave-config-host-with-scheme-and-slash.yaml")]
#[case::weave_config_missing_project("weave-config-missing-project.yaml")]
#[case::weave_preset_mappers("weave-preset-mappers.yaml")]
#[case::weave_preset_gated_without_credentials("weave-preset-gated-without-credentials.yaml")]
#[case::weave_dynamic_headers("weave-dynamic-headers.yaml")]
#[case::langtrace_preset_mapper_only("langtrace-preset-mapper-only.yaml")]
fn helper_case_matches_litellm(#[case] name: &str) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("cases/helpers/otel-presets")
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
