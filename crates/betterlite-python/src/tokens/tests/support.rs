use std::sync::Arc;

use betterlite_tokenizer::engine::Engine;
use betterlite_tokenizer::engine::huggingface::HuggingFaceEngine;

use crate::tokens::store;

pub(crate) const WORD_LEVEL: &str = r#"{"version":"1.0","truncation":null,"padding":null,"added_tokens":[],"normalizer":null,"pre_tokenizer":{"type":"Whitespace"},"post_processor":null,"decoder":null,"model":{"type":"WordLevel","vocab":{"[UNK]":0,"hello":1,"world":2},"unk_token":"[UNK]"}}"#;

pub(crate) const WITH_BOS: &str = r#"{"version":"1.0","truncation":null,"padding":null,"added_tokens":[{"id":3,"content":"<s>","single_word":false,"lstrip":false,"rstrip":false,"normalized":false,"special":true}],"normalizer":null,"pre_tokenizer":{"type":"Whitespace"},"post_processor":{"type":"TemplateProcessing","single":[{"SpecialToken":{"id":"<s>","type_id":0}},{"Sequence":{"id":"A","type_id":0}}],"pair":[{"Sequence":{"id":"A","type_id":0}}],"special_tokens":{"<s>":{"id":"<s>","ids":[3],"tokens":["<s>"]}}},"decoder":null,"model":{"type":"WordLevel","vocab":{"[UNK]":0,"hello":1,"world":2},"unk_token":"[UNK]"}}"#;

pub(crate) fn load(json: &str) -> Engine {
    Engine::HuggingFace(Arc::new(
        HuggingFaceEngine::from_bytes(json.as_bytes()).unwrap(),
    ))
}

pub(crate) fn insert(json: &str) -> u64 {
    store::insert(load(json)).unwrap()
}
