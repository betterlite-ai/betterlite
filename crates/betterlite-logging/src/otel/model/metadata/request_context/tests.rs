#![cfg(test)]
use crate::otel::model::metadata::identity::RequestContext;
use crate::otel::model::utils::coercion::OtelModelValue;

fn input(source: &str) -> OtelModelValue {
    serde_json::from_str(source).expect("test JSON is valid")
}

#[test]
fn context_preserves_request_and_provider_model_split() {
    let context = RequestContext::from_standard_logging_payload(&input(
        r#"{"model_group":"group","model":"azure/deploy","model_id":"model-id","api_base":"https://api","response":{"model":"response"}}"#,
    ));
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
    ));
    assert_eq!(context.request_model, "gpt");
    assert_eq!(context.identity.provider_model.as_deref(), Some("gpt"));
    assert_eq!(context.api_base.as_deref(), Some("hidden"));
}
