#![cfg(test)]

use pyo3::prelude::*;
use pyo3::types::PyModule;

use super::register;

pub(crate) mod support;

const FAMILY_NAMES: [&str; 15] = [
    "NativeTokensError",
    "token_count",
    "encode_text",
    "decode_tokens",
    "load_tokenizer",
    "free_tokenizer",
    "image_tokens",
    "image_tile_count",
    "image_type",
    "image_dimensions",
    "image_resize",
    "image_high_detail_bound",
    "modified_max_tokens",
    "openai_encoding_name",
    "legacy_message_accounting",
];

#[test]
fn tokens_register_exposes_the_family_names() {
    Python::attach(|py| {
        let module = PyModule::new(py, "tokens_test").unwrap();
        register(&module).unwrap();
        for name in FAMILY_NAMES {
            let function = module.getattr(name).unwrap();
            assert!(!function.is_none(), "{name} must be registered");
        }
    });
}
