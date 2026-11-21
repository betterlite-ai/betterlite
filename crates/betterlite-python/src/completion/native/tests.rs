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
                r#"{"model": "gpt-4o", "messages": [{"role": "user", "content": "hello there"}]}"#,
            )
            .unwrap();
        let counted: TokenCountResponse = serde_json::from_str(&text).unwrap();
        assert_eq!(counted.request_model, "gpt-4o");
        assert_eq!(counted.model_used, "gpt-4o");
        assert_eq!(counted.tokenizer_type, "local_tokenizer");
        assert!(counted.total_tokens > 0);
    });
}
