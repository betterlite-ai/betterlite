#![cfg(test)]

use rstest::rstest;

use super::support::{field, json, object};
use crate::gemini_live_accounting::aliases::add_usage_detail_aliases;

#[test]
fn absent_plural_details_give_zeroed_defaults() {
    let mut usage = object(r#"{"input_tokens": 1, "output_tokens": 2}"#);
    add_usage_detail_aliases(&mut usage);
    assert_eq!(
        field(&usage, "input_token_details"),
        &json(r#"{"cached_tokens": 0, "text_tokens": 0, "audio_tokens": 0}"#)
    );
    assert_eq!(
        field(&usage, "output_token_details"),
        &json(r#"{"text_tokens": 0, "audio_tokens": 0}"#)
    );
}

#[rstest]
#[case::null_plural(r#"{"input_tokens_details": null, "output_tokens_details": null}"#)]
#[case::scalar_plural(r#"{"input_tokens_details": 3, "output_tokens_details": "x"}"#)]
fn plural_details_that_are_not_objects_give_the_defaults(#[case] source: &str) {
    let mut usage = object(source);
    add_usage_detail_aliases(&mut usage);
    assert_eq!(
        field(&usage, "input_token_details"),
        &json(r#"{"cached_tokens": 0, "text_tokens": 0, "audio_tokens": 0}"#)
    );
    assert_eq!(
        field(&usage, "output_token_details"),
        &json(r#"{"text_tokens": 0, "audio_tokens": 0}"#)
    );
}

#[test]
fn non_null_plural_values_overlay_the_defaults_and_extensions_survive() {
    let mut usage = object(
        r#"{"input_tokens_details": {"audio_tokens": 7, "text_tokens": null, "cached_tokens": 2,
                "image_tokens": null, "web_search_requests": 1, "cached_tokens_details": {"text_tokens": 1}},
            "output_tokens_details": {"reasoning_tokens": 4, "audio_tokens": null, "text_tokens": 5}}"#,
    );
    add_usage_detail_aliases(&mut usage);
    assert_eq!(
        field(&usage, "input_token_details"),
        &json(
            r#"{"cached_tokens": 2, "text_tokens": 0, "audio_tokens": 7,
                "web_search_requests": 1, "cached_tokens_details": {"text_tokens": 1}}"#
        )
    );
    assert_eq!(
        field(&usage, "output_token_details"),
        &json(r#"{"text_tokens": 5, "audio_tokens": 0, "reasoning_tokens": 4}"#)
    );
}

#[test]
fn defaults_keep_their_order_ahead_of_the_overlaid_extensions() {
    let mut usage = object(r#"{"input_tokens_details": {"zeta": 1, "text_tokens": 9}}"#);
    add_usage_detail_aliases(&mut usage);
    let rendered = serde_json::to_string(field(&usage, "input_token_details")).expect("json");
    assert_eq!(
        rendered,
        r#"{"cached_tokens":0,"text_tokens":9,"audio_tokens":0,"zeta":1}"#
    );
}

#[rstest]
#[case::null(r#"{"input_token_details": null, "output_token_details": null}"#)]
#[case::object(r#"{"input_token_details": {"audio_tokens": 1}, "output_token_details": {}}"#)]
fn existing_aliases_win_even_when_null(#[case] source: &str) {
    let mut usage = object(source);
    let before = usage.clone();
    usage.insert(
        "input_tokens_details".to_owned(),
        json(r#"{"audio_tokens": 8}"#),
    );
    add_usage_detail_aliases(&mut usage);
    assert_eq!(
        field(&usage, "input_token_details"),
        field(&before, "input_token_details")
    );
    assert_eq!(
        field(&usage, "output_token_details"),
        field(&before, "output_token_details")
    );
}

#[test]
fn one_existing_alias_leaves_the_other_to_be_filled() {
    let mut usage =
        object(r#"{"input_token_details": null, "output_tokens_details": {"text_tokens": 3}}"#);
    add_usage_detail_aliases(&mut usage);
    assert_eq!(
        field(&usage, "output_token_details"),
        &json(r#"{"text_tokens": 3, "audio_tokens": 0}"#)
    );
}
