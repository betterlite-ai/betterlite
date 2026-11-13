#![cfg(test)]

use betterlite_core::api::messages_native_prepare::fields::native_filters_betas;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_prompts::arguments::json::Object;

use super::error::{BaseLlmException, MessagesContractError};
use super::{AnthropicMessagesConfig, Headers};

const NATIVE_FIELDS: [&str; 19] = [
    "messages",
    "model",
    "system",
    "max_tokens",
    "stop_sequences",
    "temperature",
    "top_p",
    "top_k",
    "tools",
    "tool_choice",
    "thinking",
    "context_management",
    "compaction",
    "output_format",
    "inference_geo",
    "speed",
    "output_config",
    "reasoning_effort",
    "safeguards",
];

const CROSS_PROVIDER_FIELDS: [&str; 18] = [
    "messages",
    "model",
    "system",
    "max_tokens",
    "stop_sequences",
    "temperature",
    "top_p",
    "top_k",
    "tools",
    "tool_choice",
    "thinking",
    "context_management",
    "output_format",
    "inference_geo",
    "speed",
    "output_config",
    "reasoning_effort",
    "safeguards",
];

struct Base;

impl AnthropicMessagesConfig for Base {
    type Error = MessagesContractError;
}

struct Anthropic;

impl AnthropicMessagesConfig for Anthropic {
    type Error = MessagesContractError;

    fn filters_beta_headers(&self) -> bool {
        native_filters_betas(self.custom_llm_provider())
    }
}

struct Bedrock;

impl AnthropicMessagesConfig for Bedrock {
    type Error = MessagesContractError;

    fn custom_llm_provider(&self) -> Option<ProviderId> {
        Some(ProviderId::Bedrock)
    }
}

fn headers(entries: &[(&str, &str)]) -> Headers {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn object(text: &str) -> Object {
    serde_json::from_str(text).expect("json object")
}

#[test]
fn native_supported_fields() {
    assert_eq!(
        Base.supported_params("claude-sonnet-4-5"),
        NATIVE_FIELDS.to_vec()
    );
    assert_eq!(
        Anthropic.supported_params("claude-sonnet-4-5"),
        NATIVE_FIELDS.to_vec()
    );
    assert_eq!(
        Bedrock.supported_params("m"),
        CROSS_PROVIDER_FIELDS.to_vec()
    );
    assert!(!Anthropic.filters_beta_headers());
    assert!(Bedrock.filters_beta_headers());
    assert!(!Base.strips_billing_metadata());
    assert!(!Anthropic.strips_billing_metadata());
    assert!(!Bedrock.strips_billing_metadata());
}

#[test]
fn base_contract_keeps_the_pinned_defaults() {
    assert!(Base.filters_beta_headers());
    assert!(!Base.strips_billing_metadata());
    assert!(Base.handles_web_search_natively());
    assert!(!Base.uses_provider_api_base());
    assert_eq!(Base.complete_url(None), "");
    assert_eq!(
        Base.complete_url(Some("https://configured.test")),
        "https://configured.test"
    );
    assert_eq!(Base.max_http_retries(), 2);
}

#[test]
fn base_contract_signs_nothing() {
    let given = headers(&[("x-caller", "value")]);
    let signed = Base.sign_request(given.clone());
    assert_eq!(signed.headers, given);
    assert_eq!(signed.signed_body, None);
}

#[test]
fn base_contract_reports_a_missing_stream_implementation() {
    let failure = Base
        .stream_iterator()
        .expect_err("no stream implementation");
    assert_eq!(
        failure,
        MessagesContractError::NotImplemented {
            message: "Subclasses must implement this method"
        }
    );
    assert_eq!(failure.to_string(), "Subclasses must implement this method");
}

#[test]
fn base_contract_keeps_the_error_fields() {
    let error = Base.error_class("teapot", 418, headers(&[("retry-after", "2")]));
    assert_eq!(
        error,
        BaseLlmException {
            status_code: 418,
            message: "teapot".to_owned(),
            headers: headers(&[("retry-after", "2")]),
        }
    );
}

#[test]
fn base_retry_defaults() {
    let invalid_signature = "messages.2.content.0: Invalid `signature` in `thinking` block";
    assert!(Base.retries_http_error(400, invalid_signature));
    assert!(!Base.retries_http_error(500, invalid_signature));
    assert!(!Base.retries_http_error(400, "bad tool"));

    let mut retried = object(
        r#"{"thinking":{"type":"enabled"},"messages":[
            {"role":"assistant","content":[{"type":"thinking","thinking":"a","signature":"s"},{"type":"text","text":"keep"}]},
            {"role":"assistant","content":[{"type":"redacted_thinking","data":"d"}]},
            {"role":"user","content":"hi"}]}"#,
    );
    Base.request_after_http_error(400, invalid_signature, &mut retried);
    assert_eq!(
        retried,
        object(
            r#"{"messages":[{"role":"assistant","content":[{"type":"text","text":"keep"}]},{"role":"user","content":"hi"}]}"#
        )
    );

    let mut untouched = object(
        r#"{"thinking":{"type":"enabled"},"messages":[{"role":"assistant","content":[{"type":"thinking","thinking":"a","signature":"s"}]}]}"#,
    );
    Base.request_after_http_error(500, invalid_signature, &mut untouched);
    assert_eq!(
        untouched,
        object(
            r#"{"thinking":{"type":"enabled"},"messages":[{"role":"assistant","content":[{"type":"thinking","thinking":"a","signature":"s"}]}]}"#
        )
    );
    Base.request_after_http_error(400, "bad tool", &mut untouched);
    assert_eq!(
        untouched,
        object(
            r#"{"thinking":{"type":"enabled"},"messages":[{"role":"assistant","content":[{"type":"thinking","thinking":"a","signature":"s"}]}]}"#
        )
    );
}
