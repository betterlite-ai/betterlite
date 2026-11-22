#![cfg(test)]

use pyo3::prelude::*;

use betterlite_sdk::completion_call::tokens::TokenCountResponse;

use super::super::register;
use super::NativeSession;

#[test]
fn register_adds_the_completion_classes() {
    Python::initialize();
    Python::attach(|py| {
        let module = PyModule::new(py, "probe").unwrap();
        register(&module).unwrap();
        assert!(module.hasattr("NativeSession").unwrap());
        assert!(module.hasattr("NativeChatStream").unwrap());
        assert!(module.hasattr("NativeError").unwrap());
    });
}

#[test]
fn a_malformed_session_text_is_an_error_not_a_panic() {
    Python::initialize();
    Python::attach(|_py| {
        assert!(NativeSession::new("not a session").is_err());
    });
}

#[test]
fn count_tokens_counts_the_messages_with_the_local_tokenizer() {
    Python::initialize();
    Python::attach(|py| {
        let session = NativeSession::new(r#"{"environment": {}}"#).unwrap();
        let text = session
            .count_tokens(
                py,
                r#"{"model": "gpt-3.5-turbo", "messages": [{"role": "user", "content": "hello world"}]}"#,
            )
            .unwrap();
        let counted: TokenCountResponse = serde_json::from_str(&text).unwrap();
        assert_eq!(counted.request_model, "gpt-3.5-turbo");
        assert_eq!(counted.model_used, "gpt-3.5-turbo");
        assert_eq!(counted.tokenizer_type, "local_tokenizer");
        // LiteLLM counts 3 + "user" + "hello world", then adds token_counter's 3 reply-priming tokens
        assert_eq!(counted.total_tokens, 9);
    });
}

#[test]
fn count_tokens_puts_the_system_prompt_first() {
    Python::initialize();
    Python::attach(|py| {
        let session = NativeSession::new(r#"{"environment": {}}"#).unwrap();
        let text = session
            .count_tokens(
                py,
                r#"{"model": "gpt-3.5-turbo", "system": "s", "messages": [{"role": "user", "content": "hello world"}]}"#,
            )
            .unwrap();
        let counted: TokenCountResponse = serde_json::from_str(&text).unwrap();
        // the system message LiteLLM prepends is 3 + "system" + "s" tokens, counted like any other message
        assert_eq!(counted.total_tokens, 14);
    });
}
