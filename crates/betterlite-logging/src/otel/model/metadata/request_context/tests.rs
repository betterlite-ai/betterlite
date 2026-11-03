#![cfg(test)]
use crate::otel::model::metadata::request_context::RequestContext;
use crate::otel::model::metadata::wire::payload::Payload;

fn input(source: &str) -> Payload {
    serde_json::from_str(source).expect("test JSON is valid")
}

#[test]
fn context_preserves_request_and_provider_model_split() {
    let context = RequestContext::from_standard_logging_payload(&input(
        r#"{"model_group":"group","model":"azure/deploy","model_id":"model-id","api_base":"https://api","response":{"model":"response"}}"#,
    )).unwrap();
    assert_eq!(context.request_model, "group");
    assert_eq!(
        context.identity.provider_model.as_deref(),
        Some("azure/deploy")
    );
    assert_eq!(context.response_model.as_deref(), Some("response"));
    assert_eq!(context.model_id.as_deref(), Some("model-id"));
    assert_eq!(context.api_base.as_deref(), Some("https://api"));
}

#[test]
fn context_falls_back_to_sdk_call_model_and_hidden_api_base() {
    let context = RequestContext::from_standard_logging_payload(&input(
        r#"{"model":"gpt","hidden_params":{"api_base":"hidden"}}"#,
    ))
    .unwrap();
    assert_eq!(context.request_model, "gpt");
    assert_eq!(context.identity.provider_model.as_deref(), Some("gpt"));
    assert_eq!(context.api_base.as_deref(), Some("hidden"));
}

#[test]
fn context_uses_metadata_fallbacks_and_empty_request_model() {
    let context = RequestContext::from_standard_logging_payload(&input(
        r#"{"metadata":{"model_group":"group","model_info":{"id":"deployment-id"}}}"#,
    ))
    .unwrap();
    assert_eq!(context.request_model, "group");
    assert_eq!(context.model_group.as_deref(), Some("group"));
    assert_eq!(context.model_id.as_deref(), Some("deployment-id"));
    let empty = RequestContext::from_standard_logging_payload(&input("{}")).unwrap();
    assert_eq!(empty.request_model, "");
    assert_eq!(empty.identity.provider_model, None);
}

#[rstest::rstest]
#[case::text(r#"{"response":"answer"}"#)]
#[case::list(r#"{"response":[{"content":"answer"}]}"#)]
fn non_object_responses_have_no_response_model(#[case] source: &str) {
    let payload: Payload = serde_json::from_str(source).unwrap();
    let context = RequestContext::from_standard_logging_payload(&payload).unwrap();
    assert_eq!(context.response_model, None);
}
