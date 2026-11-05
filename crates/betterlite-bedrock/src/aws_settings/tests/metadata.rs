#![cfg(test)]

use aws_smithy_runtime_api::{
    client::{
        http::{HttpConnector, HttpConnectorFuture, SharedHttpConnector, http_client_fn},
        orchestrator::HttpRequest,
    },
    http::{Response, StatusCode},
};
use aws_smithy_types::body::SdkBody;
use secrecy::ExposeSecret as _;

use super::support::{native, settings};
use crate::aws_settings::sources::resolve_local_credentials;

#[derive(Debug)]
struct ContainerMetadata;

impl HttpConnector for ContainerMetadata {
    fn call(&self, request: HttpRequest) -> HttpConnectorFuture {
        assert_eq!(request.uri(), "http://169.254.170.2/fixture");
        let body = SdkBody::from(
            r#"{"AccessKeyId":"container","SecretAccessKey":"private","Token":"session","Expiration":"2030-01-01T00:00:00Z"}"#,
        );
        HttpConnectorFuture::ready(Ok(Response::new(StatusCode::try_from(200).unwrap(), body)))
    }
}

#[tokio::test]
async fn container_credentials_use_injected_transport() {
    let mut source = native(
        &[],
        &[
            ("AWS_CONTAINER_CREDENTIALS_RELATIVE_URI", "/fixture"),
            ("AWS_EC2_METADATA_DISABLED", "true"),
        ],
    );
    source.client = http_client_fn(|_, _| SharedHttpConnector::new(ContainerMetadata));
    let result = resolve_local_credentials(&settings(""), &source)
        .await
        .unwrap();
    assert!(result.cache_ttl.is_none());
    let credentials = result.credentials;
    assert_eq!(credentials.access_key_id.expose_secret(), "container");
    assert_eq!(
        credentials.session_token.unwrap().expose_secret(),
        "session"
    );
}

#[tokio::test]
async fn source_errors_do_not_echo_credentials() {
    let source = native(
        &[(
            "/credentials",
            "[broken]\ncredential_process = fixture-secret-never-log\n",
        )],
        &[
            ("AWS_SHARED_CREDENTIALS_FILE", "/credentials"),
            ("AWS_EC2_METADATA_DISABLED", "true"),
        ],
    );
    let error = resolve_local_credentials(&settings("aws_profile_name: broken"), &source)
        .await
        .unwrap_err();
    assert_eq!(format!("{error:?}"), "AWS credential source failed");
    assert!(!format!("{error} {error:?}").contains("fixture-secret-never-log"));
}
