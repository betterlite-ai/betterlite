#![cfg(test)]

use aws_smithy_http_client::{
    Builder,
    tls::{Provider, rustls_provider::CryptoMode},
};

use super::{build_client, load_aws_kms};

#[tokio::test]
async fn load_aws_kms_does_not_initialize_when_disabled() {
    let transport = Builder::new()
        .tls_provider(Provider::Rustls(CryptoMode::AwsLc))
        .build_https();
    load_aws_kms(Some(false), transport, None).unwrap();
}

#[tokio::test]
async fn load_aws_kms_builds_a_client_for_the_configured_region() {
    let transport = Builder::new()
        .tls_provider(Provider::Rustls(CryptoMode::AwsLc))
        .build_https();
    let client = build_client("us-east-1".to_owned(), transport, None);
    assert_eq!(
        client.config().region().map(AsRef::as_ref),
        Some("us-east-1")
    );
}
