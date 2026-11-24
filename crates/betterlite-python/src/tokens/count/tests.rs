#![cfg(test)]

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

use super::{modified_max_tokens, token_count};
use crate::tokens::store;
use crate::tokens::tests::support::{WITH_BOS, WORD_LEVEL, insert};

#[test]
fn tokens_count_counts_a_text_like_litellm() {
    Python::attach(|py| {
        let counted = token_count(py, r#"{"model":"","text":"hello world"}"#).unwrap();
        assert_eq!(counted, 2);
    });
}

#[test]
fn tokens_count_counts_with_the_models_encoding() {
    Python::attach(|py| {
        // the CJK clause is three tokens under gpt-4o's o200k_base and six under cl100k_base
        let o200k = token_count(py, r#"{"model":"gpt-4o","text":"hello world 世界"}"#).unwrap();
        assert_eq!(o200k, 3);
        let cl100k = token_count(py, r#"{"model":"gpt-3.5-turbo","text":"hello world 世界"}"#);
        assert_eq!(cl100k.unwrap(), 6);
    });
}

#[test]
fn tokens_count_counts_through_a_custom_tokenizer() {
    let tokenizer = insert(WITH_BOS);
    Python::attach(|py| {
        let custom = format!(r#"{{"text":"hello","custom_tokenizer":{tokenizer}}}"#);
        assert_eq!(token_count(py, &custom).unwrap(), 2);
        assert_eq!(token_count(py, r#"{"text":"hello"}"#).unwrap(), 1);
    });
    store::remove(tokenizer).unwrap();
}

#[test]
fn tokens_count_counts_messages_with_litellms_overhead() {
    let tokenizer = insert(WORD_LEVEL);
    Python::attach(|py| {
        let counted = format!(
            r#"{{"custom_tokenizer":{tokenizer},"messages":[{{"role":"user","content":"hello world"}}],"count_response_tokens":false}}"#
        );
        assert_eq!(token_count(py, &counted).unwrap(), 9);
        let response = format!(
            r#"{{"custom_tokenizer":{tokenizer},"messages":[{{"role":"user","content":"hello world"}}],"count_response_tokens":true}}"#
        );
        assert_eq!(token_count(py, &response).unwrap(), 6);
    });
    store::remove(tokenizer).unwrap();
}

#[test]
fn tokens_count_follows_the_exact_threshold_from_the_environment() {
    temp_env::with_var("TOKEN_COUNTER_MAX_EXACT_CHARS", Some("1"), || {
        Python::attach(|py| {
            // a one-character sample over five characters extrapolates to five tokens, where the exact count is one
            assert_eq!(token_count(py, r#"{"text":"hello"}"#).unwrap(), 5);
        });
    });
}

#[test]
fn tokens_count_raises_litellms_value_errors() {
    Python::attach(|py| {
        let both = token_count(py, r#"{"text":"a","messages":[]}"#).unwrap_err();
        assert!(both.is_instance_of::<PyValueError>(py));
        assert!(
            both.to_string()
                .contains("text and messages cannot both be set")
        );
        let tools = token_count(
            py,
            r#"{"text":"a","tools":[{"type":"function","function":{"name":"f"}}]}"#,
        )
        .unwrap_err();
        assert!(tools.is_instance_of::<PyValueError>(py));
        assert!(
            tools
                .to_string()
                .contains("tools or tool_choice cannot be set if using text")
        );
        let choice = token_count(py, r#"{"text":"a","tool_choice":"none"}"#).unwrap_err();
        assert!(choice.is_instance_of::<PyValueError>(py));
        assert!(
            choice
                .to_string()
                .contains("tools or tool_choice cannot be set if using text")
        );
        let neither = token_count(py, "{}").unwrap_err();
        assert!(neither.is_instance_of::<PyValueError>(py));
        assert!(
            neither
                .to_string()
                .contains("Either text or messages must be provided")
        );
        let unknown = token_count(py, r#"{"text":"a","custom_tokenizer":424242}"#).unwrap_err();
        assert!(unknown.is_instance_of::<PyValueError>(py));
    });
}

#[test]
fn tokens_modified_max_tokens_returns_none_without_a_user_limit() {
    Python::attach(|py| {
        let request = r#"{"model":"gpt-4o","base_model":"gpt-4o","messages":[{"role":"user","content":"hello world"}]}"#;
        assert_eq!(modified_max_tokens(py, request).unwrap(), None);
    });
}

#[test]
fn tokens_modified_max_tokens_keeps_the_users_limit() {
    Python::attach(|py| {
        let known = r#"{"model":"gpt-4o","base_model":"gpt-4o","messages":[{"role":"user","content":"hello world"}],"user_max_tokens":55}"#;
        assert_eq!(modified_max_tokens(py, known).unwrap(), Some(55));
        let unknown = r#"{"model":"nonesuch/model-xyz","base_model":"nonesuch/model-xyz","user_max_tokens":55}"#;
        assert_eq!(modified_max_tokens(py, unknown).unwrap(), Some(55));
    });
}

#[test]
fn tokens_modified_max_tokens_shrinks_to_the_shared_context() {
    Python::attach(|py| {
        let request = r#"{"model":"ai21.j2-mid-v1","base_model":"ai21.j2-mid-v1","messages":[{"role":"user","content":"hello world"}],"user_max_tokens":20000}"#;
        assert_eq!(modified_max_tokens(py, request).unwrap(), Some(8172));
        let buffered = r#"{"model":"ai21.j2-mid-v1","base_model":"ai21.j2-mid-v1","messages":[{"role":"user","content":"hello world"}],"user_max_tokens":20000,"buffer_num":1000.0}"#;
        assert_eq!(modified_max_tokens(py, buffered).unwrap(), Some(7182));
    });
}

#[test]
fn tokens_modified_max_tokens_counts_nothing_when_the_counter_is_disabled() {
    Python::attach(|py| {
        let request = r#"{"model":"ai21.j2-mid-v1","base_model":"ai21.j2-mid-v1","messages":[{"role":"user","content":"hello world"}],"user_max_tokens":20000,"disable_token_counter":true}"#;
        assert_eq!(modified_max_tokens(py, request).unwrap(), Some(8181));
    });
}
