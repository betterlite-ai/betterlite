use std::sync::Arc;

use super::super::{ProviderHints, pick_provider};
use super::support::{
    FakeAccess, FakeProcessing, bytes_reply, caller, container_requests, file_upload_form,
    json_reply, request_parts, upload_request,
};

#[test]
fn provider_header_wins_over_query() {
    let hints = ProviderHints {
        header: Some("azure".to_owned()),
        query: Some("bedrock".to_owned()),
    };
    assert_eq!(pick_provider(&hints), "azure");
}

#[test]
fn provider_empty_header_falls_through_to_query() {
    let hints = ProviderHints {
        header: Some(String::new()),
        query: Some("bedrock".to_owned()),
    };
    assert_eq!(pick_provider(&hints), "bedrock");
}

#[test]
fn provider_empty_query_falls_through_to_default() {
    let hints = ProviderHints {
        header: Some(String::new()),
        query: Some(String::new()),
    };
    assert_eq!(pick_provider(&hints), "openai");
}

#[test]
fn provider_defaults_when_no_hint_is_set() {
    assert_eq!(pick_provider(&ProviderHints::default()), "openai");
}

#[test]
fn provider_from_request_reads_header_and_query() {
    let parts = request_parts(
        "/v1/containers/cntr/files/file-1/content?custom_llm_provider=bedrock",
        Some("azure"),
    );
    let hints = ProviderHints::from_request(&parts);
    assert_eq!(hints.header.as_deref(), Some("azure"));
    assert_eq!(hints.query.as_deref(), Some("bedrock"));
    assert_eq!(pick_provider(&hints), "azure");
}

#[tokio::test]
async fn provider_binary_request_reads_the_pick() {
    let access = Arc::new(FakeAccess::granting());
    let processing = Arc::new(FakeProcessing::replying(bytes_reply(b"png")));
    let requests = container_requests(Arc::clone(&access), processing);
    let parts = request_parts("/v1/containers/cntr/files/file-1/content", None);
    let download = requests
        .process_file_content_request(&parts, "cntr", "file-1", &caller("alice"))
        .await
        .unwrap();
    assert_eq!(download.bytes.as_slice(), b"png");
    assert_eq!(
        access.events(),
        [
            "check:cntr:alice:openai".to_owned(),
            "forward:cntr:cntr-upstream:openai".to_owned()
        ]
    );
}

#[tokio::test]
async fn provider_upload_request_reads_the_pick() {
    let access = Arc::new(FakeAccess::granting());
    let processing = Arc::new(FakeProcessing::replying(json_reply()));
    let requests = container_requests(Arc::clone(&access), processing);
    let parts = request_parts(
        "/v1/containers/cntr/files?custom_llm_provider=bedrock",
        None,
    );
    let payload = upload_request("aupload_container_file", file_upload_form());
    let reply = requests
        .process_file_upload_request(&parts, "cntr", &caller("alice"), &payload)
        .await
        .unwrap();
    assert_eq!(reply.body, json_reply().body);
    // forwarding gets the provider check_access resolved, which the fake reports as openai
    assert_eq!(
        access.events(),
        [
            "check:cntr:alice:bedrock".to_owned(),
            "forward:cntr:cntr-upstream:openai".to_owned()
        ]
    );
}
