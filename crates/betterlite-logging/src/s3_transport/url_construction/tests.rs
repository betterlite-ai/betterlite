#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::s3_transport::error::S3TransportError;
use crate::s3_transport::url_construction::{
    EndpointInputs, S3Endpoint, resolve_endpoint, resolve_region,
};

fn endpoint(endpoint_url: Option<&str>, virtual_hosted_style: bool) -> S3Endpoint {
    S3Endpoint {
        bucket: "logs".to_owned(),
        region: "us-east-1".to_owned(),
        endpoint_url: endpoint_url.map(str::to_owned),
        virtual_hosted_style,
    }
}

fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[rstest]
#[case::path_style(
    Some("http://127.0.0.1:9000"),
    false,
    "http://127.0.0.1:9000/logs/2026-01-02/id.json"
)]
#[case::path_style_keeps_its_trailing_slash(
    Some("http://minio:9000/"),
    false,
    "http://minio:9000//logs/2026-01-02/id.json"
)]
#[case::virtual_hosted(
    Some("https://s3.example.com"),
    true,
    "https://logs.s3.example.com/2026-01-02/id.json"
)]
#[case::virtual_hosted_plain_http(
    Some("http://minio:9000"),
    true,
    "http://logs.minio:9000/2026-01-02/id.json"
)]
#[case::default_aws_host(
    None,
    false,
    "https://logs.s3.us-east-1.amazonaws.com/2026-01-02/id.json"
)]
fn s3_object_urls_match_the_endpoint_shape(
    #[case] endpoint_url: Option<&str>,
    #[case] virtual_hosted_style: bool,
    #[case] expected: &str,
) {
    let url = endpoint(endpoint_url, virtual_hosted_style).object_url("2026-01-02/id.json");
    assert_eq!(url, expected);
}

#[test]
fn the_default_host_uses_the_partition_dns_suffix() {
    let china = S3Endpoint {
        bucket: "logs".to_owned(),
        region: "cn-north-1".to_owned(),
        endpoint_url: None,
        virtual_hosted_style: false,
    };
    assert_eq!(
        china.object_url("id.json"),
        "https://logs.s3.cn-north-1.amazonaws.com.cn/id.json"
    );
}

#[rstest]
#[case::space("a b", "a%20b")]
#[case::plus_and_ampersand("a+b&c", "a%2Bb%26c")]
#[case::fragment_and_query("a#b?c", "a%23b%3Fc")]
#[case::percent("a%b", "a%25b")]
#[case::equals("a=b", "a%3Db")]
#[case::at_sign("a@b", "a%40b")]
#[case::colon("a:b", "a%3Ab")]
#[case::unreserved("a-._~b", "a-._~b")]
#[case::slashes_are_safe("/a/b/", "/a/b/")]
#[case::non_ascii("\u{e9}", "%C3%A9")]
#[case::nul_byte("a\u{0}b", "a%00b")]
fn s3_object_keys_are_quoted_once_with_slash_safe(
    #[case] object_key: &str,
    #[case] expected: &str,
) {
    let url = endpoint(None, false).object_url(object_key);
    assert_eq!(
        url,
        format!("https://logs.s3.us-east-1.amazonaws.com/{expected}")
    );
}

#[rstest]
#[case::configured(Some("eu-west-1"), &[], "eu-west-1")]
#[case::region_beats_region_name(None, &[("AWS_REGION", "ap-south-1"), ("AWS_REGION_NAME", "eu-west-1")], "ap-south-1")]
#[case::region_name_fallback(None, &[("AWS_REGION_NAME", "eu-west-1")], "eu-west-1")]
#[case::empty_env_counts_as_unset(None, &[("AWS_REGION", ""), ("AWS_REGION_NAME", "eu-west-1")], "eu-west-1")]
#[case::default_region(None, &[], "us-west-2")]
fn s3_regions_resolve_configured_then_environment_then_default(
    #[case] configured: Option<&str>,
    #[case] pairs: &[(&str, &str)],
    #[case] expected: &str,
) {
    let region = resolve_region(configured, &environment(pairs)).expect("the region resolves");
    assert_eq!(region, expected);
}

#[rstest]
#[case::configured_uppercase(Some("EU-WEST-1"), &[])]
#[case::configured_empty(Some(""), &[])]
#[case::environment_with_a_space(None, &[("AWS_REGION", "eu west")])]
#[case::environment_uppercase(None, &[("AWS_REGION_NAME", "US-EAST-1")])]
fn invalid_s3_regions_are_refused(
    #[case] configured: Option<&str>,
    #[case] pairs: &[(&str, &str)],
) {
    let outcome = resolve_region(configured, &environment(pairs));
    assert!(matches!(outcome, Err(S3TransportError::Region { .. })));
}

#[test]
fn a_missing_bucket_is_refused() {
    let inputs = EndpointInputs {
        bucket: None,
        region: None,
        endpoint_url: None,
        virtual_hosted_style: false,
    };
    let outcome = resolve_endpoint(inputs, &environment(&[]));
    assert!(matches!(outcome, Err(S3TransportError::MissingBucket)));
}

#[test]
fn an_empty_bucket_is_refused() {
    let inputs = EndpointInputs {
        bucket: Some(""),
        region: None,
        endpoint_url: None,
        virtual_hosted_style: false,
    };
    let outcome = resolve_endpoint(inputs, &environment(&[]));
    assert!(matches!(outcome, Err(S3TransportError::MissingBucket)));
}

#[test]
fn endpoint_resolution_keeps_the_configured_url_and_style() {
    let inputs = EndpointInputs {
        bucket: Some("logs"),
        region: Some("eu-central-1"),
        endpoint_url: Some("http://minio:9000"),
        virtual_hosted_style: true,
    };
    let endpoint = resolve_endpoint(inputs, &environment(&[])).expect("the endpoint resolves");
    assert_eq!(
        endpoint,
        S3Endpoint {
            bucket: "logs".to_owned(),
            region: "eu-central-1".to_owned(),
            endpoint_url: Some("http://minio:9000".to_owned()),
            virtual_hosted_style: true,
        }
    );
}
