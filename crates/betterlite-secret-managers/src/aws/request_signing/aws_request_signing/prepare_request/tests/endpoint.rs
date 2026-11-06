#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use super::super::support::{FakeResolver, get_secret_request, manager, resolver, signing_time};

fn endpoint_for(
    resolver_endpoint: Option<&str>,
    environment_endpoint: Option<&str>,
    region: &str,
) -> String {
    let environment = environment_endpoint
        .map(|value| {
            BTreeMap::from([("AWS_BEDROCK_RUNTIME_ENDPOINT".to_owned(), value.to_owned())])
        })
        .unwrap_or_default();
    let credentials = FakeResolver {
        endpoint: resolver_endpoint.map(str::to_owned),
        ..resolver(region)
    };
    manager(credentials, environment)
        .prepare_request(&get_secret_request("my/secret"), None, None, signing_time())
        .unwrap()
        .endpoint_url
}

#[test]
fn the_default_endpoint_uses_the_region_and_its_partition_suffix() {
    assert_eq!(
        endpoint_for(None, None, "eu-west-1"),
        "https://secretsmanager.eu-west-1.amazonaws.com"
    );
    assert_eq!(
        endpoint_for(None, None, "cn-northwest-1"),
        "https://secretsmanager.cn-northwest-1.amazonaws.com.cn"
    );
}

#[rstest]
#[case::bedrock_host(
    "https://bedrock-runtime.us-east-1.amazonaws.com",
    "https://secretsmanager.us-east-1.amazonaws.com"
)]
#[case::every_occurrence(
    "https://bedrock-runtime.vpce.bedrock-runtime.example.test/bedrock-runtime",
    "https://secretsmanager.vpce.secretsmanager.example.test/secretsmanager"
)]
#[case::other_host_as_given("https://vpce.example.test", "https://vpce.example.test")]
#[case::case_sensitive_match(
    "https://Bedrock-Runtime.example.test",
    "https://Bedrock-Runtime.example.test"
)]
fn a_custom_endpoint_swaps_the_literal_bedrock_runtime_for_secretsmanager(
    #[case] configured: &str,
    #[case] expected: &str,
) {
    assert_eq!(endpoint_for(Some(configured), None, "us-east-1"), expected);
}

#[test]
fn the_credentials_endpoint_is_the_one_signed_and_returned() {
    let credentials = FakeResolver {
        endpoint: Some("https://bedrock-runtime.us-east-1.amazonaws.com".to_owned()),
        ..resolver("us-east-1")
    };
    let prepared = manager(credentials, BTreeMap::new())
        .prepare_request(&get_secret_request("my/secret"), None, None, signing_time())
        .unwrap();
    assert_eq!(
        prepared.headers["authorization"].to_str().unwrap(),
        "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20240305/us-east-1/secretsmanager/aws4_request, SignedHeaders=content-type;host;x-amz-date;x-amz-security-token;x-amz-target, Signature=886aa1516978326d78f46360655b9c89a78fd93a4a9b1911b41bd09c3df1155d"
    );
}

#[rstest]
#[case::environment_only(
    None,
    Some("https://bedrock-runtime.eu-west-1.example.test"),
    "https://secretsmanager.eu-west-1.example.test"
)]
#[case::credentials_win(
    Some("https://bedrock-runtime.a.example.test"),
    Some("https://bedrock-runtime.b.example.test"),
    "https://secretsmanager.a.example.test"
)]
#[case::empty_credentials_endpoint_uses_environment(
    Some(""),
    Some("https://bedrock-runtime.b.example.test"),
    "https://secretsmanager.b.example.test"
)]
#[case::empty_endpoints_use_the_default(
    Some(""),
    Some(""),
    "https://secretsmanager.us-east-1.amazonaws.com"
)]
fn the_runtime_endpoint_falls_back_to_the_injected_environment_variable(
    #[case] resolver_endpoint: Option<&str>,
    #[case] environment_endpoint: Option<&str>,
    #[case] expected: &str,
) {
    assert_eq!(
        endpoint_for(resolver_endpoint, environment_endpoint, "us-east-1"),
        expected
    );
}
