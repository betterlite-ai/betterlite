#![cfg(test)]

use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_secrets::secret_codec::OsNonces;
use betterlite_secrets::stored_cipher::app_crypto::{AppCrypto, Envelope};
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretSlice};
use serde::Deserialize;

use crate::sqs_logging::credentials::{CredentialParams, CredentialSource};
use crate::sqs_logging::health::DummyHealthPayload;
use crate::sqs_logging::settings::Settings;
use crate::sqs_logging::tests::support::{
    ACCESS_KEY_ID, FixedClock, QUEUE_URL, RecordedCredentials, RecordingDelivery,
    SECRET_ACCESS_KEY, SessionCredentials, StaticCredentials, fixed_time, logged, payload,
    settings, settings_with,
};

use super::{SendError, SqsSender};

#[derive(Deserialize)]
struct Wrapped {
    #[serde(rename = "__encrypted__")]
    encrypted: bool,
    payload: Envelope,
}

fn sender_with(
    settings: Settings,
    delivery: RecordingDelivery,
    credentials: Arc<dyn CredentialSource>,
) -> SqsSender<RecordingDelivery> {
    SqsSender {
        settings,
        delivery,
        credentials,
        clock: Arc::new(FixedClock(fixed_time())),
        health_payload: Arc::new(DummyHealthPayload),
    }
}

fn sender(settings: Settings, delivery: RecordingDelivery) -> SqsSender<RecordingDelivery> {
    sender_with(settings, delivery, Arc::new(StaticCredentials))
}

fn form_body_of(body: &str) -> String {
    let encoded = body
        .strip_prefix("Action=SendMessage&Version=2012-11-05&MessageBody=")
        .expect("the form body has the send-message prefix");
    percent_encoding::percent_decode_str(encoded)
        .decode_utf8()
        .expect("the message body is UTF-8")
        .to_string()
}

#[tokio::test]
async fn send_message_posts_the_signed_form_body() {
    let delivery = RecordingDelivery::delivering();
    let records = Arc::clone(&delivery.records);
    let sender = sender(settings(), delivery);
    sender
        .send_message(&payload())
        .await
        .expect("the message sends");
    records.wait_for(1).await;
    let entries = records.snapshot();
    let posted = &entries[0];
    assert_eq!(posted.url, QUEUE_URL);
    assert_eq!(
        posted
            .headers
            .get("content-type")
            .expect("the content type")
            .to_str()
            .expect("text"),
        "application/x-www-form-urlencoded"
    );
    assert_eq!(
        posted
            .headers
            .get("x-amz-date")
            .expect("the date")
            .to_str()
            .expect("text"),
        "20260102T030405Z"
    );
    assert!(
        posted
            .headers
            .get("authorization")
            .expect("the signature")
            .to_str()
            .expect("text")
            .starts_with("AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/")
    );
    let message = form_body_of(&String::from_utf8(posted.body.clone()).expect("UTF-8"));
    assert!(message.contains("\"id\":\"sqs-health-check\""));
}

#[tokio::test]
async fn send_message_uses_the_configured_region() {
    let delivery = RecordingDelivery::delivering();
    let records = Arc::clone(&delivery.records);
    let sender = sender(
        settings_with(&[
            ("sqs_queue_url", QUEUE_URL),
            ("sqs_region_name", "eu-west-1"),
            ("sqs_aws_access_key_id", ACCESS_KEY_ID),
            ("sqs_aws_secret_access_key", SECRET_ACCESS_KEY),
        ]),
        delivery,
    );
    sender
        .send_message(&payload())
        .await
        .expect("the message sends");
    records.wait_for(1).await;
    let entries = records.snapshot();
    let authorization = entries[0]
        .headers
        .get("authorization")
        .expect("the signature")
        .to_str()
        .expect("text");
    assert!(authorization.contains("/eu-west-1/sqs/aws4_request"));
}

#[tokio::test]
async fn send_message_signs_with_the_session_token() {
    let delivery = RecordingDelivery::delivering();
    let records = Arc::clone(&delivery.records);
    let sender = sender_with(settings(), delivery, Arc::new(SessionCredentials));
    sender
        .send_message(&payload())
        .await
        .expect("the message sends");
    records.wait_for(1).await;
    let entries = records.snapshot();
    assert_eq!(
        entries[0]
            .headers
            .get("x-amz-security-token")
            .expect("the session token")
            .to_str()
            .expect("text"),
        "SESSIONTOKEN"
    );
}

#[tokio::test]
async fn send_message_forwards_every_credential_parameter() {
    let recorded = Arc::new(RecordedCredentials::default());
    let recorded_source = Arc::clone(&recorded);
    let delivery = RecordingDelivery::delivering();
    let records = Arc::clone(&delivery.records);
    let sender = sender_with(
        settings_with(&[
            ("sqs_queue_url", QUEUE_URL),
            ("sqs_region_name", "us-east-1"),
            ("sqs_aws_access_key_id", "access"),
            ("sqs_aws_secret_access_key", "secret"),
            ("sqs_aws_session_token", "token"),
            ("sqs_aws_session_name", "session"),
            ("sqs_aws_profile_name", "profile"),
            ("sqs_aws_role_name", "role"),
            ("sqs_aws_web_identity_token", "identity"),
            ("sqs_aws_sts_endpoint", "https://sts.example.com"),
        ]),
        delivery,
        recorded_source,
    );
    sender
        .send_message(&payload())
        .await
        .expect("the message sends");
    records.wait_for(1).await;
    let calls = recorded.calls.lock().expect("the calls lock");
    assert_forwarded(&calls[0]);
}

fn assert_forwarded(forwarded: &CredentialParams) {
    assert_eq!(
        forwarded
            .access_key_id
            .as_ref()
            .expect("key")
            .expose_secret(),
        "access"
    );
    assert_eq!(
        forwarded
            .secret_access_key
            .as_ref()
            .expect("secret")
            .expose_secret(),
        "secret"
    );
    assert_eq!(
        forwarded
            .session_token
            .as_ref()
            .expect("token")
            .expose_secret(),
        "token"
    );
    assert_eq!(forwarded.region_name.as_deref(), Some("us-east-1"));
    assert_eq!(forwarded.session_name.as_deref(), Some("session"));
    assert_eq!(forwarded.profile_name.as_deref(), Some("profile"));
    assert_eq!(forwarded.role_name.as_deref(), Some("role"));
    assert_eq!(
        forwarded
            .web_identity_token
            .as_ref()
            .expect("identity")
            .expose_secret(),
        "identity"
    );
    assert_eq!(
        forwarded.sts_endpoint.as_deref(),
        Some("https://sts.example.com")
    );
}

#[tokio::test]
async fn send_message_reports_a_missing_queue_url() {
    let delivery = RecordingDelivery::delivering();
    let records = Arc::clone(&delivery.records);
    let error = sender(settings_with(&[]), delivery)
        .send_message(&payload())
        .await
        .expect_err("the queue URL is missing");
    assert!(matches!(error, SendError::MissingQueueUrl));
    assert_eq!(records.count(), 0);
}

#[tokio::test]
async fn send_message_reports_a_missing_region() {
    let delivery = RecordingDelivery::delivering();
    let error = sender(settings_with(&[("sqs_queue_url", QUEUE_URL)]), delivery)
        .send_message(&payload())
        .await
        .expect_err("the region is missing");
    assert!(matches!(error, SendError::MissingRegion));
}

#[tokio::test]
async fn send_message_reports_an_invalid_queue_url() {
    let delivery = RecordingDelivery::delivering();
    let error = sender(settings_with(&[("sqs_queue_url", "not a url")]), delivery)
        .send_message(&payload())
        .await
        .expect_err("the queue URL is invalid");
    assert!(matches!(error, SendError::QueueUrl(_)));
}

#[tokio::test]
async fn send_message_reports_a_transport_failure() {
    let error = sender(settings(), RecordingDelivery::failing())
        .send_message(&payload())
        .await
        .expect_err("the delivery fails");
    assert!(matches!(error, SendError::Transport(_)));
}

#[rstest]
#[case::at_the_error_threshold(400)]
#[case::server_error(500)]
#[tokio::test]
async fn send_message_reports_an_error_status_with_the_body(#[case] status: u16) {
    let error = sender(settings(), RecordingDelivery::answering(status, "boom"))
        .send_message(&payload())
        .await
        .expect_err("the status is an error");
    assert!(matches!(
        error,
        SendError::Status {
            status: reported,
            detail
        } if reported == status && detail == "boom"
    ));
}

#[tokio::test]
async fn send_message_describes_a_non_utf8_error_body() {
    let error = sender(
        settings(),
        RecordingDelivery::answering_bytes(500, vec![0xff, 0xfe]),
    )
    .send_message(&payload())
    .await
    .expect_err("the status is an error");
    assert!(matches!(
        error,
        SendError::Status {
            status: 500,
            detail
        } if detail == "<non-UTF-8 body>"
    ));
}

#[rstest]
#[case::redirect(302)]
#[case::below_the_error_threshold(399)]
#[tokio::test]
async fn send_message_accepts_statuses_below_the_error_threshold(#[case] status: u16) {
    sender(settings(), RecordingDelivery::answering(status, ""))
        .send_message(&payload())
        .await
        .expect("httpx does not raise below 400");
}

#[tokio::test]
async fn async_send_message_swallows_a_transport_failure() {
    let sender = sender(settings(), RecordingDelivery::failing());
    let ((), log) = logged(sender.async_send_message(&payload())).await;
    assert!(log.contains("Error sending to SQS"));
}

#[tokio::test]
async fn async_send_message_swallows_an_error_status() {
    let sender = sender(settings(), RecordingDelivery::answering(503, "unavailable"));
    let ((), log) = logged(sender.async_send_message(&payload())).await;
    assert!(log.contains("Error sending to SQS"));
    assert!(log.contains("503"));
}

#[tokio::test]
async fn send_message_encrypts_the_body_when_configured() {
    let key = STANDARD.encode([7_u8; 32]);
    let delivery = RecordingDelivery::delivering();
    let records = Arc::clone(&delivery.records);
    let sender = sender(
        settings_with(&[
            ("sqs_queue_url", QUEUE_URL),
            ("sqs_region_name", "us-east-1"),
            ("sqs_aws_access_key_id", ACCESS_KEY_ID),
            ("sqs_aws_secret_access_key", SECRET_ACCESS_KEY),
            ("sqs_aws_use_application_level_encryption", "True"),
            ("sqs_app_encryption_key_b64", &key),
        ]),
        delivery,
    );
    sender
        .send_message(&payload())
        .await
        .expect("the message sends");
    records.wait_for(1).await;
    let body = records.body(0);
    let message = form_body_of(&body);
    assert!(message.contains("\"__encrypted__\":true"));
    assert!(!message.contains("gpt-3.5-turbo"));
}

#[tokio::test]
async fn send_message_encrypts_with_the_configured_aad() {
    let key = STANDARD.encode([7_u8; 32]);
    let delivery = RecordingDelivery::delivering();
    let records = Arc::clone(&delivery.records);
    let sender = sender(
        settings_with(&[
            ("sqs_queue_url", QUEUE_URL),
            ("sqs_region_name", "us-east-1"),
            ("sqs_aws_access_key_id", ACCESS_KEY_ID),
            ("sqs_aws_secret_access_key", SECRET_ACCESS_KEY),
            ("sqs_aws_use_application_level_encryption", "True"),
            ("sqs_app_encryption_key_b64", &key),
            ("sqs_app_encryption_aad", "tenant=bill"),
        ]),
        delivery,
    );
    sender
        .send_message(&payload())
        .await
        .expect("the message sends");
    records.wait_for(1).await;
    let message = form_body_of(&records.body(0));
    let wrapped: Wrapped = serde_json::from_str(&message).expect("the envelope is JSON");
    assert!(wrapped.encrypted);
    let decrypted = AppCrypto::new(&SecretSlice::from([7_u8; 32].to_vec()), OsNonces)
        .expect("the key is 32 bytes")
        .decrypt_json(&wrapped.payload, b"tenant=bill")
        .expect("the aad matches");
    assert_eq!(
        decrypted,
        serde_json::to_value(payload()).expect("the payload serializes")
    );
}
