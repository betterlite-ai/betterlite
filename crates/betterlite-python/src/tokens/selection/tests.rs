#![cfg(test)]

use pyo3::prelude::*;

use super::{legacy_message_accounting, openai_encoding_name};

#[test]
fn tokens_selection_names_the_encoding_of_a_model() {
    Python::attach(|py| {
        let modern = openai_encoding_name(py, r#"{"model":"gpt-4o"}"#).unwrap();
        assert_eq!(modern, "o200k_base");
        let classic = openai_encoding_name(py, r#"{"model":"gpt-4"}"#).unwrap();
        assert_eq!(classic, "cl100k_base");
        let unknown = openai_encoding_name(py, r#"{"model":"nonesuch/model-xyz"}"#).unwrap();
        assert_eq!(unknown, "cl100k_base");
    });
}

// LiteLLM's own cost map has no gpt-3.5-turbo-0301 entry, so _fix_model_name normalizes it to gpt-3.5-turbo there too
#[test]
fn tokens_selection_spots_the_legacy_message_accounting() {
    Python::attach(|py| {
        let legacy = legacy_message_accounting(py, r#"{"model":"gpt-3.5-turbo-0301"}"#).unwrap();
        assert!(!legacy);
        let modern = legacy_message_accounting(py, r#"{"model":"gpt-3.5-turbo"}"#).unwrap();
        assert!(!modern);
        let unknown = legacy_message_accounting(py, r#"{"model":"nonesuch/model-xyz"}"#).unwrap();
        assert!(!unknown);
    });
}
