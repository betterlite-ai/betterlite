#![cfg(test)]

use std::collections::BTreeMap;

use http::{HeaderMap, HeaderValue};
use rstest::rstest;
use secrecy::SecretString;

use crate::completion::error::{COMPLETE_URL, CompletionError, anthropic_text_error};
use crate::completion::headers::validate_environment;
use crate::completion::model::{LegacyModel, select};

use super::{options, prompts, responses, streams};

#[rstest]
#[case::legacy_model_selection(0)]
#[case::human_assistant_prompt(1)]
#[case::legacy_options(2)]
#[case::legacy_response_stop_reason(3)]
#[case::legacy_stream_chunk(4)]
#[case::legacy_auth(5)]
#[case::legacy_error_envelope(6)]
#[tokio::test]
async fn completion_cases(#[case] case: u8) {
    match case {
        0 => legacy_model_selection(),
        1 => {
            prompts::factory_prompts().await;
            prompts::custom_prompts().await;
            prompts::custom_entries_require_every_key();
        }
        2 => {
            options::mapped_params();
            options::stop_params();
            options::request_body().await;
            options::request_defaults().await;
        }
        3 => {
            responses::completion_replaces_content().await;
            responses::empty_completion_keeps_content().await;
            responses::prompt_is_rebuilt().await;
            responses::created_clamps_a_clock_before_the_epoch().await;
            responses::failures().await;
            responses::tokenizer_choice().await;
        }
        4 => {
            streams::parsed_chunks();
            streams::stream_lines();
        }
        5 => legacy_auth(),
        _ => legacy_error_envelope(),
    }
}

fn legacy_model_selection() {
    assert_eq!(select("claude-2"), Some(LegacyModel::Claude2));
    assert_eq!(
        select("claude-instant-1"),
        Some(LegacyModel::ClaudeInstant1)
    );
    for other in [
        "Claude-2",
        "claude-2.1",
        "claude-instant-1.2",
        "anthropic/claude-2",
        " claude-2",
        "claude-2 ",
        "claude-3-5-sonnet-20241022",
        "",
    ] {
        assert_eq!(select(other), None, "{other}");
    }
}

fn legacy_auth() {
    let mut start = HeaderMap::new();
    start.insert("accept", HeaderValue::from_static("text/plain"));
    start.insert("x-custom", HeaderValue::from_static("kept"));
    let key = SecretString::from("sk-legacy");
    let headers = validate_environment(start, Some(&key)).unwrap();
    let seen = headers
        .iter()
        .map(|(name, value)| (name.as_str(), value.to_str().unwrap()))
        .collect::<BTreeMap<_, _>>();
    let expected = BTreeMap::from([
        ("accept", "application/json"),
        ("anthropic-version", "2023-06-01"),
        ("content-type", "application/json"),
        ("x-api-key", "sk-legacy"),
        ("x-custom", "kept"),
    ]);
    assert_eq!(seen, expected);
    let empty = SecretString::from("");
    let headers = validate_environment(HeaderMap::new(), Some(&empty)).unwrap();
    assert_eq!(headers["x-api-key"], "");
    let missing = validate_environment(HeaderMap::new(), None).unwrap_err();
    assert!(matches!(missing, CompletionError::MissingApiKey));
    assert_eq!(
        missing.to_string(),
        "Missing Anthropic API Key - A call is being made to anthropic but no key is set either in the environment variables or via params"
    );
    let bad = SecretString::from("line\nbreak");
    let invalid = validate_environment(HeaderMap::new(), Some(&bad));
    assert!(matches!(invalid, Err(CompletionError::InvalidApiKey)));
}

fn legacy_error_envelope() {
    let error = anthropic_text_error(418, "teapot".to_owned());
    assert_eq!(error.status_code, 418);
    assert_eq!(error.message, "teapot");
    assert!(!error.status_code_is_synthesized);
    assert_eq!(error.headers, None);
    assert_eq!(error.request.method, "POST");
    assert_eq!(error.request.url, "https://api.anthropic.com/v1/complete");
    assert_eq!(error.request.url, COMPLETE_URL);
    assert_eq!(error.response.status_code, 418);
    assert_eq!(error.response.request, error.request);
}
