use std::sync::Arc;

use betterlite_core::api::container_ownership::{ContainerForwardingParams, ContainerIdentity};
use betterlite_prompts::arguments::json::Json;

use super::super::{ContainerFileRequestError, ContainerRequestData, RequestFailure};
use super::support::{
    FakeAccess, FakeProcessing, bytes_reply, caller, container_requests, json_reply,
    mapped_exception, proxy_failure, request_parts,
};

#[tokio::test]
async fn binary_data_starts_with_file_id_and_has_no_model() {
    let mut fake_access = FakeAccess::granting();
    fake_access.identity = ContainerIdentity {
        original_container_id: "cntr-decoded".to_owned(),
        custom_llm_provider: "bedrock".to_owned(),
    };
    fake_access.forwarding = ContainerForwardingParams {
        container_id: "cntr-decoded".to_owned(),
        custom_llm_provider: "bedrock".to_owned(),
        model_id: Some("deployment-7".to_owned()),
    };
    let access = Arc::new(fake_access);
    let processing = Arc::new(FakeProcessing::replying(bytes_reply(b"png")));
    let requests = container_requests(Arc::clone(&access), Arc::clone(&processing));
    let parts = request_parts("/v1/containers/cntr/files/file-1/content", None);
    requests
        .process_file_content_request(&parts, "cntr", "file-1", &caller("alice"))
        .await
        .unwrap();
    let calls = processing.processed();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].route_type, "aretrieve_container_file_content");
    assert_eq!(calls[0].model, None);
    let ContainerRequestData::Json(object) = &calls[0].data else {
        panic!("the content data is a JSON object");
    };
    let keys: Vec<&str> = object.keys().map(String::as_str).collect();
    assert_eq!(
        keys,
        ["file_id", "container_id", "custom_llm_provider", "model_id"]
    );
    assert_eq!(
        object.get("file_id"),
        Some(&Json::Text("file-1".to_owned()))
    );
    assert_eq!(
        object.get("model_id"),
        Some(&Json::Text("deployment-7".to_owned()))
    );
}

#[tokio::test]
async fn binary_omits_model_id_when_forwarding_has_none() {
    let access = Arc::new(FakeAccess::granting());
    let processing = Arc::new(FakeProcessing::replying(bytes_reply(b"png")));
    let requests = container_requests(Arc::clone(&access), Arc::clone(&processing));
    let parts = request_parts("/v1/containers/cntr/files/file-1/content", None);
    requests
        .process_file_content_request(&parts, "cntr", "file-1", &caller("alice"))
        .await
        .unwrap();
    let calls = processing.processed();
    assert_eq!(calls.len(), 1);
    let ContainerRequestData::Json(object) = &calls[0].data else {
        panic!("the content data is a JSON object");
    };
    let keys: Vec<&str> = object.keys().map(String::as_str).collect();
    assert_eq!(keys, ["file_id", "container_id", "custom_llm_provider"]);
}

#[tokio::test]
async fn binary_returns_bytes_content_type_and_headers() {
    let access = Arc::new(FakeAccess::granting());
    let processing = Arc::new(FakeProcessing::replying(bytes_reply(b"png-bytes")));
    let requests = container_requests(Arc::clone(&access), processing);
    let parts = request_parts("/v1/containers/cntr/files/file-1.png/content", None);
    let download = requests
        .process_file_content_request(&parts, "cntr", "file-1.png", &caller("alice"))
        .await
        .unwrap();
    assert_eq!(download.bytes.as_slice(), b"png-bytes");
    assert_eq!(download.content_type, "image/png");
    let call_id = download.headers.get("x-litellm-call-id").unwrap();
    assert_eq!(call_id.to_str().unwrap(), "call-1");
}

#[tokio::test]
async fn binary_not_bytes_reply_fails_through_the_exception_handling() {
    let access = Arc::new(FakeAccess::granting());
    let fake_processing = FakeProcessing::replying(json_reply());
    *fake_processing.mapped.lock().unwrap() = Some(mapped_exception());
    let processing = Arc::new(fake_processing);
    let requests = container_requests(Arc::clone(&access), Arc::clone(&processing));
    let parts = request_parts("/v1/containers/cntr/files/file-1/content", None);
    let error = requests
        .process_file_content_request(&parts, "cntr", "file-1", &caller("alice"))
        .await
        .unwrap_err();
    let ContainerFileRequestError::Processing(exception) = error else {
        panic!("the mapped error is Processing");
    };
    assert_eq!(exception.message, "mapped failure");
    assert_eq!(
        processing.failures(),
        [RequestFailure::NotBytes {
            route_type: "aretrieve_container_file_content",
            got: "json object",
        }]
    );
    assert_eq!(
        RequestFailure::NotBytes {
            route_type: "aretrieve_container_file_content",
            got: "json object",
        }
        .to_string(),
        "aretrieve_container_file_content expected bytes, got json object"
    );
}

#[tokio::test]
async fn binary_process_failure_fails_through_the_exception_handling() {
    let access = Arc::new(FakeAccess::granting());
    let processing = Arc::new(FakeProcessing::failing(proxy_failure()));
    let requests = container_requests(Arc::clone(&access), Arc::clone(&processing));
    let parts = request_parts("/v1/containers/cntr/files/file-1/content", None);
    let error = requests
        .process_file_content_request(&parts, "cntr", "file-1", &caller("alice"))
        .await
        .unwrap_err();
    assert!(matches!(error, ContainerFileRequestError::Processing(_)));
    assert_eq!(
        processing.failures(),
        [RequestFailure::Process(proxy_failure())]
    );
}
