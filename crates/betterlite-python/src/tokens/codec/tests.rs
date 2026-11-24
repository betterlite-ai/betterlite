#![cfg(test)]

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use secrecy::SecretString;

use super::{decode_tokens, encode_text, free_tokenizer, load_tokenizer, pretrained};
use crate::tokens::error::NativeTokensError;
use crate::tokens::tests::support::WORD_LEVEL;

#[test]
fn tokens_codec_encodes_with_the_default_encoding() {
    Python::attach(|py| {
        let encoded = encode_text(py, r#"{"model":"","text":"hello world"}"#).unwrap();
        assert_eq!(encoded, [15339, 1917]);
    });
}

// LiteLLM's encode goes through _select_tokenizer, whose openai branch is always the default cl100k encoding
#[test]
fn tokens_codec_ignores_the_model_for_the_default_encoding() {
    Python::attach(|py| {
        let encoded = encode_text(py, r#"{"model":"gpt-4o","text":"hello world"}"#).unwrap();
        assert_eq!(encoded, [15339, 1917]);
    });
}

#[test]
fn tokens_codec_decodes_the_same_ids() {
    Python::attach(|py| {
        let decoded = decode_tokens(py, r#"{"tokens":[15339,1917]}"#).unwrap();
        assert_eq!(decoded, "hello world");
    });
}

#[test]
fn tokens_codec_ignores_the_special_token_flag_for_the_default_encoding() {
    Python::attach(|py| {
        let decoded =
            decode_tokens(py, r#"{"tokens":[15339,1917],"skip_special_tokens":false}"#).unwrap();
        assert_eq!(decoded, "hello world");
    });
}

#[test]
fn tokens_codec_round_trips_through_a_loaded_tokenizer() {
    let json = serde_json::to_string(WORD_LEVEL).unwrap();
    Python::attach(|py| {
        let request = format!(r#"{{"kind":"json","json":{json}}}"#);
        let tokenizer = load_tokenizer(py, &request).unwrap();
        assert!(tokenizer > 0);
        let encoding = format!(r#"{{"text":"hello world","custom_tokenizer":{tokenizer}}}"#);
        assert_eq!(encode_text(py, &encoding).unwrap(), [1, 2]);
        let decoding = format!(r#"{{"tokens":[1,2],"custom_tokenizer":{tokenizer}}}"#);
        assert_eq!(decode_tokens(py, &decoding).unwrap(), "hello world");
        assert_eq!(free_tokenizer(tokenizer).unwrap(), ());
        let freed = encode_text(py, &encoding).unwrap_err();
        assert!(freed.is_instance_of::<PyValueError>(py));
        assert_eq!(free_tokenizer(tokenizer).unwrap(), ());
    });
}

#[test]
fn tokens_codec_rejects_keeping_special_tokens_on_a_hugging_face_tokenizer() {
    let json = serde_json::to_string(WORD_LEVEL).unwrap();
    Python::attach(|py| {
        let request = format!(r#"{{"kind":"json","json":{json}}}"#);
        let tokenizer = load_tokenizer(py, &request).unwrap();
        let keeping = format!(
            r#"{{"tokens":[1,2],"custom_tokenizer":{tokenizer},"skip_special_tokens":false}}"#
        );
        let error = decode_tokens(py, &keeping).unwrap_err();
        assert!(error.is_instance_of::<NativeTokensError>(py));
        free_tokenizer(tokenizer).unwrap();
    });
}

#[test]
fn tokens_codec_keeps_the_pretrained_identifier_revision_and_token() {
    let custom = pretrained("Xenova/x", "v1", Some(SecretString::from("secret")));
    assert_eq!(custom.identifier, "Xenova/x");
    assert_eq!(custom.revision, "v1");
    assert!(custom.auth_token.is_some());
    let anonymous = pretrained("Xenova/x", "main", None);
    assert!(anonymous.auth_token.is_none());
}

#[test]
fn tokens_codec_pretrained_load_reports_the_hub_error() {
    let home = tempfile::tempdir().unwrap();
    let home_text = home.path().to_str().unwrap();
    temp_env::with_vars(
        [("HF_HUB_OFFLINE", Some("1")), ("HF_HOME", Some(home_text))],
        || {
            Python::attach(|py| {
                let request =
                    r#"{"kind":"pretrained","identifier":"betterlite-nonexistent/repository"}"#;
                let error = load_tokenizer(py, request).unwrap_err();
                let text = error.to_string();
                assert!(text.contains("local cache"), "{text}");
                assert!(text.contains("betterlite-nonexistent/repository"), "{text}");
            });
        },
    );
}
