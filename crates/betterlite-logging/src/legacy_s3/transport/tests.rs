#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use secrecy::{ExposeSecret as _, SecretString};

use crate::s3_transport::encryption::ServerSideEncryption;
use crate::s3_transport::error::S3TransportError;
use crate::s3_transport::transport::S3Transport;

use super::build;
use crate::legacy_s3::settings::Settings;

fn client() -> ProxiedClient {
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new())
            .expect("the proxy policy resolves"),
    );
    ProxiedClient::outbound(&policy).expect("the HTTP client builds")
}

fn environment() -> BTreeMap<String, String> {
    BTreeMap::new()
}

fn settings() -> Settings {
    Settings {
        bucket_name: Some("logs".to_owned()),
        path: String::new(),
        region_name: Some("us-east-1".to_owned()),
        endpoint_url: None,
        access_key_id: Some(SecretString::from("access")),
        secret_access_key: Some(SecretString::from("secret")),
        session_token: None,
        encryption: None,
        use_team_prefix: false,
        log_prompts_only: None,
        preview_features: false,
    }
}

fn built(settings: &Settings) -> S3Transport {
    build(settings, Arc::new(environment()), client())
        .expect("the transport builds")
        .expect("a bucket is configured")
}

#[test]
fn returns_no_transport_without_a_bucket() {
    let mut configured = settings();
    configured.bucket_name = None;
    let transport = build(&configured, Arc::new(environment()), client())
        .expect("the missing bucket is allowed");
    assert!(transport.is_none());
}

#[test]
fn builds_the_aws_virtual_hosted_url() {
    let transport = built(&settings());
    assert_eq!(transport.endpoint.bucket, "logs");
    assert_eq!(transport.endpoint.region, "us-east-1");
    assert_eq!(
        transport.endpoint.object_url("key.json"),
        "https://logs.s3.us-east-1.amazonaws.com/key.json"
    );
}

#[test]
fn keeps_a_custom_endpoint_path_style() {
    let mut configured = settings();
    configured.endpoint_url = Some("http://localhost:9000".to_owned());
    let transport = built(&configured);
    assert_eq!(
        transport.endpoint.object_url("key.json"),
        "http://localhost:9000/logs/key.json"
    );
}

#[test]
fn resolves_the_region_from_the_environment() {
    let mut configured = settings();
    configured.region_name = None;
    let variables: BTreeMap<String, String> =
        BTreeMap::from([("AWS_REGION_NAME".to_owned(), "eu-central-1".to_owned())]);
    let transport = build(&configured, Arc::new(variables), client())
        .expect("the transport builds")
        .expect("a bucket is configured");
    assert_eq!(transport.endpoint.region, "eu-central-1");
}

#[test]
fn refuses_an_invalid_region() {
    let mut configured = settings();
    configured.region_name = Some("US_EAST_1".to_owned());
    let error =
        build(&configured, Arc::new(environment()), client()).expect_err("the region is invalid");
    assert!(matches!(error, S3TransportError::Region { .. }));
}

#[test]
fn refuses_an_invalid_endpoint_url() {
    let mut configured = settings();
    configured.endpoint_url = Some("not a url".to_owned());
    let error =
        build(&configured, Arc::new(environment()), client()).expect_err("the endpoint is invalid");
    assert!(matches!(error, S3TransportError::Url { .. }));
}

#[tokio::test]
async fn credentials_use_the_configured_keys() {
    let transport = built(&settings());
    let credentials = transport
        .credentials
        .resolve()
        .await
        .expect("the credentials resolve");
    assert_eq!(credentials.access_key_id.expose_secret(), "access");
    assert_eq!(credentials.secret_access_key.expose_secret(), "secret");
}

#[test]
fn carries_the_configured_encryption() {
    let mut configured = settings();
    configured.encryption = Some(ServerSideEncryption {
        algorithm: "AES256".to_owned(),
        kms_key_id: None,
    });
    let transport = built(&configured);
    assert_eq!(
        transport.encryption.expect("encryption is set").algorithm,
        "AES256"
    );
}
