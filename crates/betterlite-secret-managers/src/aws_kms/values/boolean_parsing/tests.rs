#![cfg(test)]

use crate::aws_kms::{
    AwsKmsError, DecryptedValue,
    initialization::build_client,
    tests::{credentials, kms_environment, transport, with_kms},
    values::{decrypt_value, decrypt_with, parse_plaintext},
};
use aws_sdk_kms::error::SdkError;
use rstest::rstest;
use secrecy::ExposeSecret as _;
use std::{collections::BTreeMap, error::Error as _, time::Duration};
use tokio::{net::TcpListener, time::timeout};

#[rstest]
#[case("true")]
#[case("TRUE")]
#[case("false")]
#[case(" FALSE ")]
#[case("1")]
#[case("(True)")]
#[case("True # flag")]
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
    assert!(matches!(
        parse_plaintext("\u{1c}\u{a0}True\u{1f}\n"),
        DecryptedValue::Boolean(true)
    ));
}

#[tokio::test]
async fn decrypt_with_extracts_secret_plaintext() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client = build_client(
        &kms_environment(&listener, "us-east-1"),
        transport(),
        Some(credentials()),
        Duration::from_secs(5),
    )
    .unwrap();
    let decrypt = Box::pin(decrypt_with(&client, "aGVsbG8="));
    let (result, requests) =
        with_kms(listener, 200, r#"{"Plaintext":"IHNlY3JldCA="}"#, decrypt).await;
    let DecryptedValue::String(secret) = result.unwrap() else {
        panic!("expected secret");
    };
    assert_eq!(secret.expose_secret(), "secret");
    let [request] = requests.as_slice() else {
        panic!("expected one request");
    };
    assert_eq!(request.body, r#"{"CiphertextBlob":"aGVsbG8="}"#);
}

#[rstest]
#[case::missing("{}", "missing")]
#[case::invalid_utf8(r#"{"Plaintext":"/w=="}"#, "utf8")]
#[case::bad_json("{", "request")]
#[tokio::test]
async fn decrypt_with_preserves_response_errors(#[case] body: &str, #[case] expected: &str) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client = build_client(
        &kms_environment(&listener, "us-east-1"),
        transport(),
        Some(credentials()),
        Duration::from_secs(5),
    )
    .unwrap();
    let (result, _requests) = with_kms(
        listener,
        200,
        body,
        Box::pin(decrypt_with(&client, "aGVsbG8=")),
    )
    .await;
    let error = result.err().unwrap();
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
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client = build_client(
        &kms_environment(&listener, "us-east-1"),
        transport(),
        Some(credentials()),
        Duration::from_millis(100),
    )
    .unwrap();
    let result = timeout(Duration::from_secs(5), decrypt_with(&client, "aGVsbG8="))
        .await
        .unwrap();
    drop(listener);
    assert!(matches!(
        result,
        Err(AwsKmsError::Request(source)) if matches!(source.as_ref(), SdkError::TimeoutError(_))
    ));
}

#[tokio::test]
async fn decrypt_value_requires_the_named_environment_entry() {
    let environment = BTreeMap::from([("AWS_REGION_NAME".to_owned(), "us-east-1".to_owned())]);
    let client = build_client(&environment, transport(), None, Duration::from_secs(1)).unwrap();
    assert!(
        matches!(decrypt_value(&client, &environment, "TOKEN").await, Err(AwsKmsError::MissingValue(name)) if name == "TOKEN")
    );
}

#[tokio::test]
async fn decrypt_with_keeps_service_error_source() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client = build_client(
        &kms_environment(&listener, "us-east-1"),
        transport(),
        Some(credentials()),
        Duration::from_secs(5),
    )
    .unwrap();
    let (result, _requests) = with_kms(
        listener,
        400,
        r#"{"__type":"InvalidCiphertextException","message":"invalid ciphertext"}"#,
        Box::pin(decrypt_with(&client, "aGVsbG8=")),
    )
    .await;
    let error = result.err().unwrap();
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
