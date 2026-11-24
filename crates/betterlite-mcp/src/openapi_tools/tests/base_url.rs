use rstest::rstest;

use crate::openapi_tools::base_url::get_base_url;

use super::support::document;

#[rstest]
#[case::absolute_server_url(
    r#"{"servers":[{"url":"https://api.example.com/v1"}]}"#,
    None,
    "https://api.example.com/v1"
)]
#[case::first_server_wins(
    r#"{"servers":[{"url":"https://one.test"},{"url":"https://two.test"}]}"#,
    None,
    "https://one.test"
)]
#[case::relative_url_with_an_http_spec(
    r#"{"servers":[{"url":"/api/v3"}]}"#,
    Some("https://petstore.test/v3/openapi.json"),
    "https://petstore.test/api/v3"
)]
#[case::relative_url_with_a_port_and_userinfo(
    r#"{"servers":[{"url":"/api"}]}"#,
    Some("https://user:pass@host.test:8443/spec.json"),
    "https://user:pass@host.test:8443/api"
)]
#[case::relative_url_with_a_local_spec_stays_relative(
    r#"{"servers":[{"url":"/api/v3"}]}"#,
    Some("/tmp/openapi.json"),
    "/api/v3"
)]
#[case::relative_url_without_a_spec_path(r#"{"servers":[{"url":"/api/v3"}]}"#, None, "/api/v3")]
#[case::server_url_without_a_leading_slash(
    r#"{"servers":[{"url":"api.example.com"}]}"#,
    Some("https://petstore.test/openapi.json"),
    "api.example.com"
)]
#[case::server_without_a_url(r#"{"servers":[{}]}"#, Some("https://petstore.test/openapi.json"), "")]
#[case::empty_servers_fall_through_to_host(
    r#"{"servers":[],"host":"swagger.test"}"#,
    Some("https://loader.test/openapi.json"),
    "https://swagger.test"
)]
fn reads_openapi3_servers(
    #[case] spec: &str,
    #[case] spec_path: Option<&str>,
    #[case] expected: &str,
) {
    assert_eq!(get_base_url(&document(spec), spec_path), expected);
}

#[rstest]
#[case::host_with_scheme_and_base_path(
    r#"{"host":"swagger.test","schemes":["http"],"basePath":"/v2"}"#,
    "http://swagger.test/v2"
)]
#[case::host_without_schemes_uses_https(r#"{"host":"swagger.test"}"#, "https://swagger.test")]
#[case::first_scheme_wins(
    r#"{"host":"swagger.test","schemes":["http","https"],"basePath":"/v1"}"#,
    "http://swagger.test/v1"
)]
#[case::empty_schemes_list_uses_https(
    r#"{"host":"swagger.test","schemes":[]}"#,
    "https://swagger.test"
)]
#[case::empty_host_is_kept(r#"{"host":""}"#, "https://")]
fn reads_swagger2_host(#[case] spec: &str, #[case] expected: &str) {
    assert_eq!(
        get_base_url(&document(spec), Some("https://loader.test/spec.json")),
        expected
    );
}

#[rstest]
#[case::openapi_json("https://api.test/v1/openapi.json", "https://api.test/v1")]
#[case::openapi_yaml("https://api.test/v1/openapi.yaml", "https://api.test/v1")]
#[case::swagger_json("https://api.test/v1/swagger.json", "https://api.test/v1")]
#[case::swagger_yaml("https://api.test/v1/swagger.yaml", "https://api.test/v1")]
#[case::other_json_drops_the_last_segment("https://api.test/v1/spec.json", "https://api.test/v1")]
#[case::yaml_drops_the_last_segment("https://api.test/v1/spec.yaml", "https://api.test/v1")]
#[case::yml_drops_the_last_segment("https://api.test/v1/spec.yml", "https://api.test/v1")]
#[case::unknown_suffix_derives_nothing("https://api.test/v1/spec.txt", "")]
#[case::a_file_above_the_root_keeps_its_origin("https://api.test/spec.json", "https://api.test")]
fn derives_the_base_from_the_spec_location(#[case] spec_path: &str, #[case] expected: &str) {
    assert_eq!(get_base_url(&document("{}"), Some(spec_path)), expected);
}

#[test]
fn a_local_spec_location_derives_nothing() {
    assert_eq!(get_base_url(&document("{}"), Some("/tmp/openapi.json")), "");
}

#[test]
fn no_location_and_no_server_derives_nothing() {
    assert_eq!(get_base_url(&document("{}"), None), "");
}
