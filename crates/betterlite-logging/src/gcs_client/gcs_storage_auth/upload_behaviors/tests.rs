#![cfg(test)]

use betterlite_models::decisions::Json;

use super::log_json_data_on_gcs;
use crate::gcs_client::gcs_storage_auth::error::GcsClientError;
use crate::gcs_client::gcs_storage_auth::tests::support::{
    Harness, RecordingClient, headers, logging_payload,
};
use crate::gcs_mock::request::GcsRequest;
use crate::types::gcs_storage_auth::type_definitions::GcsLoggingPayload;

const UPLOAD_URL: &str =
    "https://storage.googleapis.com/upload/storage/v1/b/bucket/o?uploadType=media&name=object";

fn harness_with(client: RecordingClient) -> Harness {
    let mut harness = Harness::new(&[("GCS_BUCKET_NAME", "bucket")]);
    harness.client = client;
    harness
}

fn sent_post(harness: &Harness) -> (String, Json) {
    let request = harness.last_request();
    let url = request.url().to_owned();
    let GcsRequest::Post(write) = request else {
        panic!("the upload sent a POST");
    };
    (url, write.data.expect("the upload carries a body"))
}

#[tokio::test]
async fn a_text_payload_is_posted_unchanged() {
    let harness = harness_with(RecordingClient::responding(200, "{}"));
    let base = harness.build(None);
    let payload = GcsLoggingPayload::Text("{\"id\":\"one\"}".to_owned());
    log_json_data_on_gcs(&base, &headers(&[]), "bucket", "object", &payload)
        .await
        .expect("the fake client answers");
    let (url, data) = sent_post(&harness);
    assert_eq!(url, UPLOAD_URL);
    assert_eq!(data, Json::Text("{\"id\":\"one\"}".to_owned()));
}

#[tokio::test]
async fn a_standard_payload_is_serialized_to_json() {
    let harness = harness_with(RecordingClient::responding(200, "{}"));
    let base = harness.build(None);
    let payload = GcsLoggingPayload::Standard(Box::new(logging_payload()));
    log_json_data_on_gcs(&base, &headers(&[]), "bucket", "object", &payload)
        .await
        .expect("the fake client answers");
    let (_url, data) = sent_post(&harness);
    let Json::Text(text) = data else {
        panic!("the upload body is text");
    };
    let decoded: Json = serde_json::from_str(&text).expect("the body is JSON");
    let Json::Object(fields) = decoded else {
        panic!("the body is a JSON object");
    };
    assert_eq!(fields.get("id"), Some(&Json::Text("chatcmpl-1".to_owned())));
    assert_eq!(fields.get("model"), Some(&Json::Text("gpt-4o".to_owned())));
    assert_eq!(
        fields.get("total_tokens"),
        Some(&Json::Number(serde_json::Number::from(10_i32)))
    );
}

#[tokio::test]
async fn the_object_name_is_encoded_in_the_upload_url() {
    let harness = harness_with(RecordingClient::responding(200, "{}"));
    let base = harness.build(None);
    let payload = GcsLoggingPayload::Text("body".to_owned());
    log_json_data_on_gcs(&base, &headers(&[]), "bucket", "a b.json", &payload)
        .await
        .expect("the fake client answers");
    let (url, _data) = sent_post(&harness);
    assert_eq!(
        url,
        "https://storage.googleapis.com/upload/storage/v1/b/bucket/o?uploadType=media&name=a%20b.json"
    );
}

#[tokio::test]
async fn a_folder_bucket_puts_the_folder_in_the_object_path() {
    let harness = harness_with(RecordingClient::responding(200, "{}"));
    let base = harness.build(None);
    let payload = GcsLoggingPayload::Text("body".to_owned());
    log_json_data_on_gcs(
        &base,
        &headers(&[]),
        "my-bucket/folder/dev",
        "object",
        &payload,
    )
    .await
    .expect("the fake client answers");
    let (url, _data) = sent_post(&harness);
    assert_eq!(
        url,
        "https://storage.googleapis.com/upload/storage/v1/b/my-bucket/o?uploadType=media&name=folder%2Fdev%2Fobject"
    );
}

#[tokio::test]
async fn the_upload_request_carries_the_given_headers() {
    let harness = harness_with(RecordingClient::responding(200, "{}"));
    let base = harness.build(None);
    let payload = GcsLoggingPayload::Text("body".to_owned());
    let request_headers = headers(&[("Authorization", "Bearer token")]);
    log_json_data_on_gcs(&base, &request_headers, "bucket", "object", &payload)
        .await
        .expect("the fake client answers");
    let request = harness.last_request();
    let GcsRequest::Post(write) = request else {
        panic!("the upload sent a POST");
    };
    assert_eq!(
        write
            .headers
            .get("authorization")
            .map(|value| value.to_str().unwrap_or_default()),
        Some("Bearer token")
    );
}

#[tokio::test]
async fn the_parsed_response_is_returned() {
    let harness = harness_with(RecordingClient::responding(
        200,
        "{\"kind\":\"storage#object\",\"name\":\"stored\"}",
    ));
    let base = harness.build(None);
    let payload = GcsLoggingPayload::Text("body".to_owned());
    let response = log_json_data_on_gcs(&base, &headers(&[]), "bucket", "object", &payload)
        .await
        .expect("the fake client answers");
    let Json::Object(fields) = response else {
        panic!("the response is a JSON object");
    };
    assert_eq!(
        fields.get("kind"),
        Some(&Json::Text("storage#object".to_owned()))
    );
}

#[tokio::test]
async fn a_non_200_upload_still_returns_the_parsed_response() {
    let harness = harness_with(RecordingClient::responding(
        500,
        "{\"error\":{\"code\":500}}",
    ));
    let base = harness.build(None);
    let payload = GcsLoggingPayload::Text("body".to_owned());
    let response = log_json_data_on_gcs(&base, &headers(&[]), "bucket", "object", &payload)
        .await
        .expect("the fake client answers");
    let Json::Object(fields) = response else {
        panic!("the response is a JSON object");
    };
    assert!(fields.contains_key("error"));
}

#[tokio::test]
async fn a_response_that_is_not_json_is_an_error() {
    let harness = harness_with(RecordingClient::responding(200, "not json"));
    let base = harness.build(None);
    let payload = GcsLoggingPayload::Text("body".to_owned());
    let result = log_json_data_on_gcs(&base, &headers(&[]), "bucket", "object", &payload).await;
    assert!(matches!(result, Err(GcsClientError::Response(_))));
}

#[tokio::test]
async fn a_failed_transport_is_an_error() {
    let harness = harness_with(RecordingClient::failing());
    let base = harness.build(None);
    let payload = GcsLoggingPayload::Text("body".to_owned());
    let result = log_json_data_on_gcs(&base, &headers(&[]), "bucket", "object", &payload).await;
    assert!(matches!(result, Err(GcsClientError::Transport(_))));
}

#[tokio::test]
async fn a_bucket_that_is_not_a_name_is_an_error() {
    let harness = harness_with(RecordingClient::responding(200, "{}"));
    let base = harness.build(None);
    let payload = GcsLoggingPayload::Text("body".to_owned());
    let result =
        log_json_data_on_gcs(&base, &headers(&[]), "gs://bucket", "object", &payload).await;
    assert!(matches!(result, Err(GcsClientError::Bucket(_))));
    assert!(
        harness.requests().is_empty(),
        "the client was asked to store the object"
    );
}

#[tokio::test]
async fn a_header_that_is_not_valid_is_an_error() {
    let harness = harness_with(RecordingClient::responding(200, "{}"));
    let base = harness.build(None);
    let payload = GcsLoggingPayload::Text("body".to_owned());
    let request_headers = headers(&[("Authorization", "Bearer\r\nInjected: yes")]);
    let result = log_json_data_on_gcs(&base, &request_headers, "bucket", "object", &payload).await;
    assert!(matches!(result, Err(GcsClientError::Header(_))));
}

#[tokio::test]
async fn the_mock_upload_answers_with_the_canned_object() {
    let harness = Harness::new(&[("GCS_MOCK", "true"), ("GCS_MOCK_LATENCY_MS", "0")]);
    let base = harness.build(None);
    let payload = GcsLoggingPayload::Text("body".to_owned());
    let response = log_json_data_on_gcs(&base, &headers(&[]), "bucket", "object", &payload)
        .await
        .expect("the mock client answers");
    let Json::Object(fields) = response else {
        panic!("the mock response is a JSON object");
    };
    assert_eq!(
        fields.get("name"),
        Some(&Json::Text("mock-object".to_owned()))
    );
    assert!(harness.requests().is_empty(), "the real client was reached");
}
