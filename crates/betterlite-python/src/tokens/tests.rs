#![cfg(test)]

use pyo3::prelude::*;
use pyo3::types::PyModule;
use rstest::rstest;

use super::register;

pub(crate) mod support;

#[rstest]
#[case::native_tokens_error("NativeTokensError")]
#[case::token_count("token_count")]
#[case::encode_text("encode_text")]
#[case::decode_tokens("decode_tokens")]
#[case::load_tokenizer("load_tokenizer")]
#[case::free_tokenizer("free_tokenizer")]
#[case::image_tokens("image_tokens")]
#[case::image_tile_count("image_tile_count")]
#[case::image_type("image_type")]
#[case::image_dimensions("image_dimensions")]
#[case::image_resize("image_resize")]
#[case::image_high_detail_bound("image_high_detail_bound")]
#[case::modified_max_tokens("modified_max_tokens")]
#[case::openai_encoding_name("openai_encoding_name")]
#[case::legacy_message_accounting("legacy_message_accounting")]
fn tokens_register_exposes_the_family_names(#[case] name: &str) {
    Python::attach(|py| {
        let module = PyModule::new(py, "tokens_test").unwrap();
        register(&module).unwrap();
        module.getattr(name).unwrap();
    });
}
