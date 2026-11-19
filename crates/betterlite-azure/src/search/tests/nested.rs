use crate::search::AzureSearchConfig;
use crate::search::error::AzureSearchError;
use crate::search::{DOCS_URL, UPSTREAM_ERROR_STATUS};

use super::support::{header_pairs, model_settings, reply, transform};

pub(super) fn nested_upstream_error() {
    inner_messages_are_unwrapped();
    invalid_layers_keep_their_text();
    trailing_punctuation_is_trimmed();
    failed_status_preserves_status_and_headers();
    schema_failure_uses_the_raw_status();
}

fn failed_body(message: &str) -> String {
    let quoted = serde_json::to_string(message).unwrap();
    format!(r#"{{"output":[],"status":"failed","error":{{"message":{quoted}}}}}"#)
}

fn message_of(config: &AzureSearchConfig<'_>, body: &str) -> String {
    let error = transform(config, body, 200, &[]).unwrap_err();
    assert_eq!(error.status_code(), Some(UPSTREAM_ERROR_STATUS));
    error.to_string()
}

fn inner_messages_are_unwrapped() {
    let settings = model_settings();
    let config = AzureSearchConfig {
        settings: &settings,
        minter: None,
    };
    let single = failed_body(r#"{"error": {"message": "connection not found. "}}"#);
    assert_eq!(
        message_of(&config, &single),
        format!("Grounding with Bing Search: connection not found. See {DOCS_URL} for details.")
    );
    let double = failed_body(
        r#"{"error": {"message": "{\"error\": \"Tool_User_Error\", \"message\": \"deep detail\", \"code\": \"invalid_tool_input\"}"}}"#,
    );
    assert_eq!(
        message_of(&config, &double),
        format!("Grounding with Bing Search: deep detail. See {DOCS_URL} for details.")
    );
    let no_inner_message = failed_body(r#"{"error": {"message": "{\"error\": null}"}}"#);
    assert_eq!(
        message_of(&config, &no_inner_message),
        format!("Grounding with Bing Search: {{\"error\": null}}. See {DOCS_URL} for details.")
    );
}

fn invalid_layers_keep_their_text() {
    let settings = model_settings();
    let config = AzureSearchConfig {
        settings: &settings,
        minter: None,
    };
    let broken = failed_body("{oops");
    assert_eq!(
        message_of(&config, &broken),
        format!("Grounding with Bing Search: {{oops. See {DOCS_URL} for details.")
    );
    let no_error = failed_body(r#"{"error": null}"#);
    assert_eq!(
        message_of(&config, &no_error),
        format!("Grounding with Bing Search: {{\"error\": null}}. See {DOCS_URL} for details.")
    );
    let empty_inner = failed_body(r#"{"error": {"message": "{\"message\": \"\"}"}}"#);
    assert_eq!(
        message_of(&config, &empty_inner),
        format!("Grounding with Bing Search: {{\"message\": \"\"}}. See {DOCS_URL} for details.")
    );
    let empty_message = failed_body("");
    assert_eq!(
        message_of(&config, &empty_message),
        format!(
            "Grounding with Bing Search: the grounded search failed. See {DOCS_URL} for details."
        )
    );
}

fn trailing_punctuation_is_trimmed() {
    let settings = model_settings();
    let config = AzureSearchConfig {
        settings: &settings,
        minter: None,
    };
    let body = failed_body("plain failure...  ");
    assert_eq!(
        message_of(&config, &body),
        format!("Grounding with Bing Search: plain failure. See {DOCS_URL} for details.")
    );
}

fn failed_status_preserves_status_and_headers() {
    let settings = model_settings();
    let config = AzureSearchConfig {
        settings: &settings,
        minter: None,
    };
    let pairs = header_pairs(&[("x-request-id", "abc123"), ("Retry-After", "7")]);
    let body = failed_body("upstream failed");
    let failed = reply(&body, 200, &pairs);
    let error = config.transform_search_response(&failed, None).unwrap_err();
    let AzureSearchError::Provider {
        status_code,
        headers,
        detail,
    } = error
    else {
        panic!("the failed status did not become a provider error");
    };
    assert_eq!(status_code, UPSTREAM_ERROR_STATUS);
    assert_eq!(detail, "upstream failed");
    assert_eq!(
        headers.get("x-request-id").map(String::as_str),
        Some("abc123")
    );
    assert_eq!(headers.get("retry-after").map(String::as_str), Some("7"));
}

fn schema_failure_uses_the_raw_status() {
    let settings = model_settings();
    let config = AzureSearchConfig {
        settings: &settings,
        minter: None,
    };
    let pairs = header_pairs(&[("x-request-id", "abc123"), ("Retry-After", "7")]);
    let unparsed = reply("{}", 429, &pairs);
    let error = config
        .transform_search_response(&unparsed, None)
        .unwrap_err();
    let AzureSearchError::Provider {
        status_code,
        headers,
        detail,
    } = error
    else {
        panic!("the schema failure did not become a provider error");
    };
    assert_eq!(status_code, 429);
    assert!(detail.contains("response does not match the Foundry Responses API schema"));
    assert_eq!(headers.len(), 2_usize);
}
