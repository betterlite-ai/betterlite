use betterlite_openai::search::records::SearchResponse;

use crate::search::error::AzureSearchError;
use crate::search::{AzureSearchConfig, DOCS_URL, UPSTREAM_ERROR_STATUS};

use super::support::{model_settings, transform};

pub(super) fn empty_search_response() {
    empty_outputs_succeed();
    non_2xx_envelopes_are_reported_by_their_body();
    failed_status_always_raises();
    incomplete_without_results_raises();
    incomplete_with_results_succeeds();
    the_envelope_requires_output();
}

fn non_2xx_envelopes_are_reported_by_their_body() {
    let settings = model_settings();
    let config = AzureSearchConfig {
        settings: &settings,
        minter: None,
    };
    let empty = transform(&config, r#"{"output": []}"#, 500, &[]).unwrap();
    assert!(empty.results.is_empty());
    let body = r#"{"output":[{"type":"message","content":[{"type":"output_text","text":"kept","annotations":[{"type":"url_citation","url":"https://a.example"}]}]}]}"#;
    let found = transform(&config, body, 404, &[]).unwrap();
    assert_eq!(found.results.len(), 1_usize);
}

fn empty_outputs_succeed() {
    let settings = model_settings();
    let config = AzureSearchConfig {
        settings: &settings,
        minter: None,
    };
    let empty = transform(&config, r#"{"output": []}"#, 200, &[]).unwrap();
    assert!(empty.results.is_empty());
    assert_eq!(empty.object.0, "search");
    let completed = r#"{"output":[],"status":"completed","id":"resp-1","error":null,"incomplete_details":null,"metadata":{"a":1}}"#;
    let response: SearchResponse = transform(&config, completed, 200, &[]).unwrap();
    assert!(response.results.is_empty());
}

fn failed_status_always_raises() {
    let settings = model_settings();
    let config = AzureSearchConfig {
        settings: &settings,
        minter: None,
    };
    let with_message =
        r#"{"output":[],"status":"failed","error":{"message":"bing connection is down."}}"#;
    let error = transform(&config, with_message, 200, &[]).unwrap_err();
    assert_eq!(error.status_code(), Some(UPSTREAM_ERROR_STATUS));
    assert_eq!(
        error.to_string(),
        format!("Grounding with Bing Search: bing connection is down. See {DOCS_URL} for details.")
    );
    let without_message = r#"{"output":[{"type":"message","content":[{"type":"output_text","text":"ignored","annotations":[{"type":"url_citation","url":"https://a.example"}]}]}],"status":"failed"}"#;
    let without_message_error = transform(&config, without_message, 200, &[]).unwrap_err();
    assert_eq!(
        without_message_error.to_string(),
        format!(
            "Grounding with Bing Search: the grounded search failed. See {DOCS_URL} for details."
        )
    );
}

fn incomplete_without_results_raises() {
    let settings = model_settings();
    let config = AzureSearchConfig {
        settings: &settings,
        minter: None,
    };
    let reasoned = r#"{"output":[],"status":"incomplete","incomplete_details":{"reason":"max_output_tokens"}}"#;
    assert_incomplete(&config, reasoned, "max_output_tokens");
    assert_incomplete(
        &config,
        r#"{"output": [], "status": "incomplete"}"#,
        "unknown reason",
    );
    let empty_reason = r#"{"output":[],"status":"incomplete","incomplete_details":{"reason":""}}"#;
    assert_incomplete(&config, empty_reason, "unknown reason");
}

fn error_for(config: &AzureSearchConfig<'_>, body: &str, status: u16) -> AzureSearchError {
    transform(config, body, status, &[]).unwrap_err()
}

fn assert_incomplete(config: &AzureSearchConfig<'_>, body: &str, reason: &str) {
    let error = error_for(config, body, 200);
    assert_eq!(error.status_code(), Some(UPSTREAM_ERROR_STATUS));
    assert_eq!(
        error.to_string(),
        format!(
            "Grounding with Bing Search: the grounded search was incomplete: {reason}. See {DOCS_URL} for details."
        )
    );
}

fn incomplete_with_results_succeeds() {
    let settings = model_settings();
    let config = AzureSearchConfig {
        settings: &settings,
        minter: None,
    };
    let body = r#"{"output":[{"type":"message","content":[{"type":"output_text","text":"kept","annotations":[{"type":"url_citation","url":"https://a.example"}]}]}],"status":"incomplete","incomplete_details":{"reason":"max_output_tokens"}}"#;
    let response = transform(&config, body, 200, &[]).unwrap();
    assert_eq!(response.results.len(), 1_usize);
}

fn the_envelope_requires_output() {
    let settings = model_settings();
    let config = AzureSearchConfig {
        settings: &settings,
        minter: None,
    };
    assert_schema_failure(&config, r#"{"status": "completed"}"#, 200);
    assert_schema_failure(&config, "{}", 200);
    assert_schema_failure(&config, r#"{"output": null}"#, 200);
    assert_schema_failure(&config, "[]", 200);
    assert_schema_failure(&config, "not-json", 200);
    assert_schema_failure(&config, "{}", 500);
    assert_schema_failure(&config, "not-json", 429);
}

fn assert_schema_failure(config: &AzureSearchConfig<'_>, body: &str, status: u16) {
    let error = error_for(config, body, status);
    assert_eq!(error.status_code(), Some(status));
    let message = error.to_string();
    assert!(message.contains("response does not match the Foundry Responses API schema"));
}
