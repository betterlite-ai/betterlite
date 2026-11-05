#![cfg(test)]

use crate::aws_kms::DecryptedValue;
use crate::aws_kms::values::parse_plaintext;
use rstest::rstest;
use tokio::time::timeout;

#[rstest]
#[case("true")]
#[case("TRUE")]
#[case("false")]
#[case(" FALSE ")]
#[case("1")]
fn boolean_parsing_only_accepts_python_case_sensitive_literals(#[case] plaintext: &str) {
    assert!(matches!(
        parse_plaintext(plaintext),
        DecryptedValue::String(_)
    ));
}

#[test]
fn python_boolean_literals_become_boolean_values() {
    assert!(matches!(
        parse_plaintext(" True "),
        DecryptedValue::Boolean(true)
    ));
    assert!(matches!(
        parse_plaintext("False"),
        DecryptedValue::Boolean(false)
    ));
}

#[tokio::test]
async fn decrypt_with_extracts_secret_plaintext() {
    use crate::aws_kms::{initialization::build_client, tests::replay, values::decrypt_with};
    use aws_sdk_kms::config::{Credentials, SharedCredentialsProvider, SharedHttpClient};
    use secrecy::ExposeSecret as _;
    use std::time::Duration;
    let transport = replay(r#"{"Plaintext":"IHNlY3JldCA="}"#, 200);
    let client = build_client(
        "us-east-1".to_owned(),
        SharedHttpClient::new(transport.clone()),
        Some(SharedCredentialsProvider::new(Credentials::new(
            "test", "test", None, None, "test",
        ))),
        Duration::from_secs(1),
    );
    let DecryptedValue::String(secret) = decrypt_with(&client, "aGVsbG8=").await.unwrap() else {
        panic!("expected secret");
    };
    assert_eq!(secret.expose_secret(), "secret");
    assert!(transport.response.lock().unwrap().is_none());
}

#[rstest]
#[case::missing("{}", "missing")]
#[case::invalid_utf8(r#"{"Plaintext":"/w=="}"#, "utf8")]
#[case::bad_json("{", "request")]
#[tokio::test]
async fn decrypt_with_preserves_response_errors(#[case] body: &str, #[case] expected: &str) {
    use crate::aws_kms::{
        AwsKmsError, initialization::build_client, tests::replay, values::decrypt_with,
    };
    use aws_sdk_kms::config::{Credentials, SharedCredentialsProvider, SharedHttpClient};
    use std::{error::Error as _, time::Duration};
    let client = build_client(
        "us-east-1".to_owned(),
        SharedHttpClient::new(replay(body, 200)),
        Some(SharedCredentialsProvider::new(Credentials::new(
            "test", "test", None, None, "test",
        ))),
        Duration::from_secs(1),
    );
    let error = decrypt_with(&client, "aGVsbG8=").await.err().unwrap();
    match expected {
        "missing" => assert!(matches!(error, AwsKmsError::MissingPlaintext)),
        "utf8" => assert!(matches!(error, AwsKmsError::Utf8(_))),
        "request" => {
            assert!(matches!(error, AwsKmsError::Request(_)));
            assert!(error.source().is_some());
        }
        _ => panic!("unknown case"),
    }
}

#[tokio::test]
async fn decrypt_with_times_out_a_stalled_transport() {
    use crate::aws_kms::tests::Replay;
    use crate::aws_kms::{AwsKmsError, initialization::build_client, values::decrypt_with};
    use aws_sdk_kms::{
        config::{Credentials, SharedCredentialsProvider, SharedHttpClient},
        error::SdkError,
    };
    use std::sync::{Arc, Mutex};
    use std::time::Duration;
    let client = build_client(
        "us-east-1".to_owned(),
        SharedHttpClient::new(Replay {
            response: Arc::new(Mutex::new(None)),
        }),
        Some(SharedCredentialsProvider::new(Credentials::new(
            "test", "test", None, None, "test",
        ))),
        Duration::from_millis(10),
    );
    let result = timeout(Duration::from_secs(1), decrypt_with(&client, "aGVsbG8="))
        .await
        .unwrap();
    assert!(matches!(
        result,
        Err(AwsKmsError::Request(source)) if matches!(source.as_ref(), SdkError::TimeoutError(_))
    ));
}

#[tokio::test]
async fn decrypt_value_requires_the_named_environment_entry() {
    use crate::aws_kms::{
        AwsKmsError, initialization::build_client, tests::replay, values::decrypt_value,
    };
    use aws_sdk_kms::config::SharedHttpClient;
    use std::{collections::BTreeMap, time::Duration};
    let client = build_client(
        "us-east-1".to_owned(),
        SharedHttpClient::new(replay("{}", 200)),
        None,
        Duration::from_secs(1),
    );
    assert!(
        matches!(decrypt_value(&client, &BTreeMap::new(), "TOKEN").await, Err(AwsKmsError::MissingValue(name)) if name == "TOKEN")
    );
}

#[tokio::test]
async fn decrypt_with_keeps_service_error_source() {
    use crate::aws_kms::{
        AwsKmsError, initialization::build_client, tests::replay, values::decrypt_with,
    };
    use aws_sdk_kms::config::{Credentials, SharedCredentialsProvider, SharedHttpClient};
    use std::{error::Error as _, time::Duration};
    let transport = replay(
        r#"{"__type":"InvalidCiphertextException","message":"invalid ciphertext"}"#,
        400,
    );
    let client = build_client(
        "us-east-1".to_owned(),
        SharedHttpClient::new(transport),
        Some(SharedCredentialsProvider::new(Credentials::new(
            "test", "test", None, None, "test",
        ))),
        Duration::from_secs(1),
    );
    let error = decrypt_with(&client, "aGVsbG8=").await.err().unwrap();
    assert!(error.source().is_some());
    let AwsKmsError::Request(source) = error else {
        panic!("expected SDK error");
    };
    assert!(
        source
            .as_service_error()
            .unwrap()
            .is_invalid_ciphertext_exception()
    );
}
