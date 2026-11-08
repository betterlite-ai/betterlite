#![cfg(test)]

use std::collections::BTreeMap;
use std::num::NonZeroUsize;

use rstest::rstest;

use crate::loader::ConfigLoader;
use crate::sections::Section as _;
use crate::sections::token_counting::{Raw, Settings};

// this section shares litellm_settings with every other one, so a key it does not read must still surface as unknown
#[test]
fn unknown_litellm_keys_stay_rejected() {
    let error = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load("litellm_settings: {disable_token_counter: true, not_a_setting: 1}")
    .unwrap_err();
    assert!(error.to_string().contains("not_a_setting"));
}

#[rstest]
#[case::absent("litellm_settings: {}", &[], false, 4_000_000, 1024)]
#[case::disabled_true("litellm_settings: {disable_token_counter: true}", &[], true, 4_000_000, 1024)]
#[case::disabled_false("litellm_settings: {disable_token_counter: false}", &[], false, 4_000_000, 1024)]
#[case::disabled_null("litellm_settings: {disable_token_counter: null}", &[], false, 4_000_000, 1024)]
#[case::exact_configured("litellm_settings: {}", &[("TOKEN_COUNTER_MAX_EXACT_CHARS", "64")], false, 64, 1024)]
#[case::exact_lowest("litellm_settings: {}", &[("TOKEN_COUNTER_MAX_EXACT_CHARS", "1")], false, 1, 1024)]
#[case::exact_highest("litellm_settings: {}", &[("TOKEN_COUNTER_MAX_EXACT_CHARS", "1000000000")], false, 1_000_000_000, 1024)]
#[case::exact_zero("litellm_settings: {}", &[("TOKEN_COUNTER_MAX_EXACT_CHARS", "0")], false, 4_000_000, 1024)]
#[case::exact_above_range("litellm_settings: {}", &[("TOKEN_COUNTER_MAX_EXACT_CHARS", "1000000001")], false, 4_000_000, 1024)]
#[case::exact_negative("litellm_settings: {}", &[("TOKEN_COUNTER_MAX_EXACT_CHARS", "-3")], false, 4_000_000, 1024)]
#[case::exact_not_a_number("litellm_settings: {}", &[("TOKEN_COUNTER_MAX_EXACT_CHARS", "many")], false, 4_000_000, 1024)]
#[case::exact_empty("litellm_settings: {}", &[("TOKEN_COUNTER_MAX_EXACT_CHARS", "")], false, 4_000_000, 1024)]
#[case::exact_padded("litellm_settings: {}", &[("TOKEN_COUNTER_MAX_EXACT_CHARS", " 128 ")], false, 128, 1024)]
#[case::chunk_configured("litellm_settings: {}", &[("TIKTOKEN_ENCODE_CHUNK_SIZE_CHARS", "4")], false, 4_000_000, 4)]
#[case::chunk_lowest("litellm_settings: {}", &[("TIKTOKEN_ENCODE_CHUNK_SIZE_CHARS", "1")], false, 4_000_000, 1)]
#[case::chunk_highest("litellm_settings: {}", &[("TIKTOKEN_ENCODE_CHUNK_SIZE_CHARS", "4096")], false, 4_000_000, 4096)]
#[case::chunk_zero("litellm_settings: {}", &[("TIKTOKEN_ENCODE_CHUNK_SIZE_CHARS", "0")], false, 4_000_000, 1024)]
#[case::chunk_above_range("litellm_settings: {}", &[("TIKTOKEN_ENCODE_CHUNK_SIZE_CHARS", "4097")], false, 4_000_000, 1024)]
#[case::chunk_not_a_number("litellm_settings: {}", &[("TIKTOKEN_ENCODE_CHUNK_SIZE_CHARS", "wide")], false, 4_000_000, 1024)]
#[case::both_configured(
    "litellm_settings: {disable_token_counter: true}",
    &[("TOKEN_COUNTER_MAX_EXACT_CHARS", "32"), ("TIKTOKEN_ENCODE_CHUNK_SIZE_CHARS", "8")],
    true,
    32,
    8
)]
fn settings_follow_litellm(
    #[case] yaml: &str,
    #[case] variables: &[(&str, &str)],
    #[case] disabled: bool,
    #[case] max_exact_chars: usize,
    #[case] chunk_chars: usize,
) {
    let environment: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    let raw: Raw = serde_saphyr::from_str(yaml).unwrap();
    let settings = raw.convert(&environment).unwrap();
    assert_eq!(settings.disable_token_counter, disabled);
    assert_eq!(
        settings.max_exact_chars,
        NonZeroUsize::new(max_exact_chars).unwrap()
    );
    assert_eq!(
        settings.chunk_chars,
        NonZeroUsize::new(chunk_chars).unwrap()
    );
}

#[test]
fn the_loader_accepts_the_disable_key() {
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load("litellm_settings: {disable_token_counter: true}")
    .unwrap();
    assert!(config.sections.token_counting.disable_token_counter);
}

#[test]
fn unset_settings_match_defaults() {
    assert_eq!(
        Settings::default(),
        Settings {
            disable_token_counter: false,
            max_exact_chars: NonZeroUsize::new(4_000_000).unwrap(),
            chunk_chars: NonZeroUsize::new(1024).unwrap(),
        }
    );
}
