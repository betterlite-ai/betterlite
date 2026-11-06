#![cfg(test)]

use std::collections::BTreeMap;
use std::path::Path;

use rstest::rstest;

use crate::sections::Section as _;
use crate::sections::tokenizer_engines::{Raw, Settings};

#[rstest]
#[case::absent("litellm_settings: {}", &[], None, None, 4)]
#[case::settings_null("litellm_settings: null", &[], None, None, 4)]
#[case::disable_null("litellm_settings: {disable_hf_tokenizer_download: null}", &[], None, None, 4)]
#[case::disabled_true("litellm_settings: {disable_hf_tokenizer_download: true}", &[], Some(true), None, 4)]
#[case::disabled_false("litellm_settings: {disable_hf_tokenizer_download: false}", &[], Some(false), None, 4)]
#[case::counts_configured("litellm_settings: {}", &[("TOKEN_COUNTER_MAX_CONCURRENT_COUNTS", "8")], None, None, 8)]
#[case::counts_padded("litellm_settings: {}", &[("TOKEN_COUNTER_MAX_CONCURRENT_COUNTS", " 16 ")], None, None, 16)]
#[case::counts_zero("litellm_settings: {}", &[("TOKEN_COUNTER_MAX_CONCURRENT_COUNTS", "0")], None, None, 4)]
#[case::counts_above_range("litellm_settings: {}", &[("TOKEN_COUNTER_MAX_CONCURRENT_COUNTS", "257")], None, None, 4)]
#[case::counts_negative("litellm_settings: {}", &[("TOKEN_COUNTER_MAX_CONCURRENT_COUNTS", "-2")], None, None, 4)]
#[case::counts_not_a_number("litellm_settings: {}", &[("TOKEN_COUNTER_MAX_CONCURRENT_COUNTS", "lots")], None, None, 4)]
#[case::counts_empty("litellm_settings: {}", &[("TOKEN_COUNTER_MAX_CONCURRENT_COUNTS", "")], None, None, 4)]
#[case::cache_dir("litellm_settings: {}", &[("CUSTOM_TIKTOKEN_CACHE_DIR", "/cache/tokens")], None, Some("/cache/tokens"), 4)]
#[case::cache_dir_empty("litellm_settings: {}", &[("CUSTOM_TIKTOKEN_CACHE_DIR", "")], None, None, 4)]
fn settings_follow_litellm(
    #[case] yaml: &str,
    #[case] variables: &[(&str, &str)],
    #[case] disable_download: Option<bool>,
    #[case] cache_dir: Option<&str>,
    #[case] max_concurrent_counts: usize,
) {
    let environment: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    let raw: Raw = serde_saphyr::from_str(yaml).unwrap();
    let settings = raw.convert(&environment).unwrap();
    assert_eq!(settings.disable_hf_tokenizer_download, disable_download);
    assert_eq!(
        settings.custom_tiktoken_cache_dir.as_deref(),
        cache_dir.map(Path::new)
    );
    assert_eq!(settings.max_concurrent_counts, max_concurrent_counts);
}

#[test]
fn unset_settings_match_defaults() {
    assert_eq!(
        Settings::default(),
        Settings {
            disable_hf_tokenizer_download: None,
            custom_tiktoken_cache_dir: None,
            max_concurrent_counts: 4,
        }
    );
}
