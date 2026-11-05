#![cfg(test)]

use std::collections::BTreeMap;
use std::str::from_utf8;

use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::value::RawValue;

use super::super::support::{
    FakeResolver, ResolverFailure, get_secret_request, manager, resolver, signing_time,
};
use crate::aws::request_signing::{ActionRequest, AwsRequestError, PreparedRequest};

const GET_SESSION_SIGNATURE: &str = "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20240305/us-east-1/secretsmanager/aws4_request, SignedHeaders=content-type;host;x-amz-date;x-amz-security-token;x-amz-target, Signature=886aa1516978326d78f46360655b9c89a78fd93a4a9b1911b41bd09c3df1155d";

fn prepare_get(resolver: FakeResolver) -> PreparedRequest {
    manager(resolver, BTreeMap::new())
        .prepare_request(&get_secret_request("my/secret"), None, None, signing_time())
        .unwrap()
}

#[test]
fn signs_get_secret_value_like_botocore_with_a_session_token() {
    let prepared = prepare_get(resolver("us-east-1"));
    assert_eq!(
        prepared.endpoint_url,
        "https://secretsmanager.us-east-1.amazonaws.com"
    );
    assert_eq!(
        from_utf8(prepared.body.expose_secret()).unwrap(),
        r#"{"SecretId":"my/secret"}"#
    );
    assert_eq!(
        prepared.headers["authorization"].to_str().unwrap(),
        GET_SESSION_SIGNATURE
    );
    assert_eq!(
        prepared.headers["x-amz-date"].to_str().unwrap(),
        "20240305T123045Z"
    );
    assert_eq!(
        prepared.headers["x-amz-security-token"].to_str().unwrap(),
        "session-token"
    );
    assert_eq!(
        prepared.headers["content-type"].to_str().unwrap(),
        "application/x-amz-json-1.1"
    );
    assert_eq!(
        prepared.headers["x-amz-target"].to_str().unwrap(),
        "secretsmanager.GetSecretValue"
    );
}

#[test]
fn signs_without_a_session_token_header_when_the_credentials_have_none() {
    let prepared = prepare_get(FakeResolver {
        has_session_token: false,
        ..resolver("us-east-1")
    });
    assert!(!prepared.headers.contains_key("x-amz-security-token"));
    assert_eq!(
        prepared.headers["authorization"].to_str().unwrap(),
        "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20240305/us-east-1/secretsmanager/aws4_request, SignedHeaders=content-type;host;x-amz-date;x-amz-target, Signature=57ee46ec7a6226be5b9d8b5925a87e495c2e3e1ac23d0217b3a818e2d944d266"
    );
}

#[test]
fn signs_with_the_region_the_credentials_resolved_to() {
    let prepared = prepare_get(resolver("cn-north-1"));
    assert_eq!(
        prepared.endpoint_url,
        "https://secretsmanager.cn-north-1.amazonaws.com.cn"
    );
    assert_eq!(
        prepared.headers["authorization"].to_str().unwrap(),
        "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20240305/cn-north-1/secretsmanager/aws4_request, SignedHeaders=content-type;host;x-amz-date;x-amz-security-token;x-amz-target, Signature=db4123162ce8a10a821a539fe55b42c5256ba10599cf84863210afb6f2a1f5ec"
    );
}

#[test]
fn put_secret_value_adds_the_secret_string_to_the_default_body() {
    let secret_value = SecretString::from("s3cret");
    let request = ActionRequest {
        action: "PutSecretValue",
        ..get_secret_request("my/secret")
    };
    let prepared = manager(resolver("us-east-1"), BTreeMap::new())
        .prepare_request(&request, Some(&secret_value), None, signing_time())
        .unwrap();
    assert_eq!(
        from_utf8(prepared.body.expose_secret()).unwrap(),
        r#"{"SecretId":"my/secret","SecretString":"s3cret"}"#
    );
    assert_eq!(
        prepared.headers["x-amz-target"].to_str().unwrap(),
        "secretsmanager.PutSecretValue"
    );
    assert_eq!(
        prepared.headers["authorization"].to_str().unwrap(),
        "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20240305/us-east-1/secretsmanager/aws4_request, SignedHeaders=content-type;host;x-amz-date;x-amz-security-token;x-amz-target, Signature=c326d5882dfdd3f619e424f490d9512bb99f7d9389022aeb06c45062b9337317"
    );
}

#[rstest]
#[case::get_secret_value("GetSecretValue", "s3cret")]
#[case::create_secret("CreateSecret", "s3cret")]
#[case::put_with_empty_value("PutSecretValue", "")]
fn the_default_body_leaves_out_the_secret_string_unless_put_has_a_value(
    #[case] action: &str,
    #[case] value: &str,
) {
    let secret_value = SecretString::from(value);
    let request = ActionRequest {
        action,
        ..get_secret_request("my/secret")
    };
    let prepared = manager(resolver("us-east-1"), BTreeMap::new())
        .prepare_request(&request, Some(&secret_value), None, signing_time())
        .unwrap();
    assert_eq!(
        from_utf8(prepared.body.expose_secret()).unwrap(),
        r#"{"SecretId":"my/secret"}"#
    );
}

#[test]
fn put_secret_value_without_a_value_sends_only_the_secret_id() {
    let request = ActionRequest {
        action: "PutSecretValue",
        ..get_secret_request("my/secret")
    };
    let prepared = manager(resolver("us-east-1"), BTreeMap::new())
        .prepare_request(&request, None, None, signing_time())
        .unwrap();
    assert_eq!(
        from_utf8(prepared.body.expose_secret()).unwrap(),
        r#"{"SecretId":"my/secret"}"#
    );
}

#[test]
fn non_empty_request_data_replaces_the_default_body() {
    let mut data = BTreeMap::new();
    data.insert(
        "SecretId".to_owned(),
        RawValue::from_string(r#""other""#.to_owned()).unwrap(),
    );
    data.insert(
        "ReplicaRegions".to_owned(),
        RawValue::from_string(r#"[{"Region":"us-west-2"}]"#.to_owned()).unwrap(),
    );
    let request = ActionRequest {
        request_data: Some(&data),
        ..get_secret_request("my/secret")
    };
    let secret_value = SecretString::from("ignored");
    let prepared = manager(resolver("us-east-1"), BTreeMap::new())
        .prepare_request(&request, Some(&secret_value), None, signing_time())
        .unwrap();
    assert_eq!(
        from_utf8(prepared.body.expose_secret()).unwrap(),
        r#"{"ReplicaRegions":[{"Region":"us-west-2"}],"SecretId":"other"}"#
    );
}

#[test]
fn empty_request_data_falls_back_to_the_default_body() {
    let data = BTreeMap::new();
    let request = ActionRequest {
        request_data: Some(&data),
        ..get_secret_request("my/secret")
    };
    let prepared = manager(resolver("us-east-1"), BTreeMap::new())
        .prepare_request(&request, None, None, signing_time())
        .unwrap();
    assert_eq!(
        from_utf8(prepared.body.expose_secret()).unwrap(),
        r#"{"SecretId":"my/secret"}"#
    );
}

#[test]
fn the_target_header_names_the_requested_action() {
    let request = ActionRequest {
        action: "DescribeSecret",
        ..get_secret_request("my/secret")
    };
    let prepared = manager(resolver("us-east-1"), BTreeMap::new())
        .prepare_request(&request, None, None, signing_time())
        .unwrap();
    assert_eq!(
        prepared.headers["x-amz-target"].to_str().unwrap(),
        "secretsmanager.DescribeSecret"
    );
    assert!(
        prepared.headers["authorization"]
            .to_str()
            .unwrap()
            .contains(
                "SignedHeaders=content-type;host;x-amz-date;x-amz-security-token;x-amz-target"
            )
    );
}

#[test]
fn a_credential_failure_surfaces_unchanged() {
    let failing = FakeResolver {
        fail: true,
        ..resolver("us-east-1")
    };
    let error = manager(failing, BTreeMap::new())
        .prepare_request(&get_secret_request("my/secret"), None, None, signing_time())
        .unwrap_err();
    assert!(matches!(
        error,
        AwsRequestError::Credentials(ResolverFailure::Unavailable)
    ));
}

#[test]
fn an_unusable_endpoint_is_a_signing_error() {
    let broken = FakeResolver {
        endpoint: Some("not a url".to_owned()),
        ..resolver("us-east-1")
    };
    let error = manager(broken, BTreeMap::new())
        .prepare_request(&get_secret_request("my/secret"), None, None, signing_time())
        .unwrap_err();
    assert!(matches!(error, AwsRequestError::Signing(_)));
}

#[test]
fn credential_headers_are_sensitive_and_the_date_is_not() {
    let prepared = prepare_get(resolver("us-east-1"));
    assert!(prepared.headers["authorization"].is_sensitive());
    assert!(prepared.headers["x-amz-security-token"].is_sensitive());
    assert!(!prepared.headers["x-amz-date"].is_sensitive());
}
