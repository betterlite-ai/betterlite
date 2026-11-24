#![cfg(test)]

use std::sync::Arc;

use betterlite_tokenizer::engine::Engine;
use betterlite_tokenizer::engine::huggingface::HuggingFaceEngine;

use super::{engine, insert, remove};
use crate::tokens::error::TokensError;

const WORD_LEVEL: &str = r#"{"version":"1.0","truncation":null,"padding":null,"added_tokens":[],"normalizer":null,"pre_tokenizer":{"type":"Whitespace"},"post_processor":null,"decoder":null,"model":{"type":"WordLevel","vocab":{"[UNK]":0,"hello":1,"world":2},"unk_token":"[UNK]"}}"#;

const WITH_BOS: &str = r#"{"version":"1.0","truncation":null,"padding":null,"added_tokens":[{"id":3,"content":"<s>","single_word":false,"lstrip":false,"rstrip":false,"normalized":false,"special":true}],"normalizer":null,"pre_tokenizer":{"type":"Whitespace"},"post_processor":{"type":"TemplateProcessing","single":[{"SpecialToken":{"id":"<s>","type_id":0}},{"Sequence":{"id":"A","type_id":0}}],"pair":[{"Sequence":{"id":"A","type_id":0}}],"special_tokens":{"<s>":{"id":"<s>","ids":[3],"tokens":["<s>"]}}},"decoder":null,"model":{"type":"WordLevel","vocab":{"[UNK]":0,"hello":1,"world":2},"unk_token":"[UNK]"}}"#;

fn word_level() -> Engine {
    Engine::HuggingFace(Arc::new(
        HuggingFaceEngine::from_bytes(WORD_LEVEL.as_bytes()).unwrap(),
    ))
}

fn with_bos() -> Engine {
    Engine::HuggingFace(Arc::new(
        HuggingFaceEngine::from_bytes(WITH_BOS.as_bytes()).unwrap(),
    ))
}

#[test]
fn tokens_store_keeps_an_engine_until_it_is_freed() {
    let tokenizer = insert(word_level()).unwrap();
    let stored = engine(tokenizer).unwrap();
    assert_eq!(stored.encode("hello world").unwrap(), [1, 2]);
    remove(tokenizer).unwrap();
}

#[test]
fn tokens_store_gives_every_engine_its_own_handle() {
    let first = insert(word_level()).unwrap();
    let second = insert(with_bos()).unwrap();
    assert_ne!(first, second);
    assert_eq!(engine(first).unwrap().encode("hello").unwrap(), [1]);
    assert_eq!(engine(second).unwrap().encode("hello").unwrap(), [3, 1]);
}

#[test]
fn tokens_store_never_hands_out_zero() {
    let tokenizer = insert(word_level()).unwrap();
    assert!(tokenizer > 0);
    remove(tokenizer).unwrap();
}

#[test]
fn tokens_store_refuses_an_unknown_handle() {
    let error = engine(u64::MAX).unwrap_err();
    assert!(matches!(
        error,
        TokensError::UnknownTokenizer { tokenizer } if tokenizer == u64::MAX
    ));
}

#[test]
fn tokens_store_forgets_a_freed_handle() {
    let tokenizer = insert(word_level()).unwrap();
    remove(tokenizer).unwrap();
    let error = engine(tokenizer).unwrap_err();
    assert!(matches!(
        error,
        TokensError::UnknownTokenizer { tokenizer: freed } if freed == tokenizer
    ));
    remove(tokenizer).unwrap();
}

#[test]
fn tokens_store_decodes_with_the_litellm_skip_default() {
    let tokenizer = insert(with_bos()).unwrap();
    let stored = engine(tokenizer).unwrap();
    assert_eq!(stored.encode("hello").unwrap(), [3, 1]);
    let text = stored.decode(&[3, 1]).unwrap();
    assert!(!text.contains("<s>"));
    remove(tokenizer).unwrap();
}
