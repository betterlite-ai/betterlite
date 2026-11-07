use crate::spend_sanitize::raw_model::{scrub_raw_model_from_errors, scrub_raw_model_text};
use crate::spend_sanitize::truncate::TRUNCATION_DB_SAFEGUARD_NOTE;
use crate::spend_sanitize::value::SpendValue;
use betterlite_models::decisions::Json;

use super::support::{lookup, value};

const MODEL: &str = "a'b\\c\u{e9}";

fn marker(skipped: u32) -> String {
    format!("... (litellm_truncated skipped {skipped} chars. {TRUNCATION_DB_SAFEGUARD_NOTE}) ...")
}

#[test]
fn replaces_literal_repr_and_json_spellings_with_the_unknown_model() {
    let text = "a'b\\c\u{e9} matches a'b\\\\c\u{e9} and a'b\\\\c\\u00e9";
    assert_eq!(
        scrub_raw_model_text(text, MODEL),
        "unknown-model matches unknown-model and unknown-model"
    );
}

#[test]
fn deduplicates_spellings_that_are_identical() {
    assert_eq!(
        scrub_raw_model_text("gpt-4o calls gpt-4o", "gpt-4o"),
        "unknown-model calls unknown-model"
    );
}

#[test]
fn removes_the_longest_prefix_and_suffix_around_a_truncation_marker() {
    let head = "request for a'b\\c";
    let tail = "\u{e9}-tail and a'b\\c\u{e9}";
    let text = format!("{head}{}{tail}", marker(12));
    let expected = format!(
        "request for unknown-model{}unknown-model-tail and unknown-model",
        marker(12)
    );
    assert_eq!(scrub_raw_model_text(&text, MODEL), expected);
}

#[test]
fn leaves_the_text_alone_when_no_marker_matches() {
    let text = format!(
        "a'b\\c\u{e9} ... (litellm_truncated skipped x chars. {TRUNCATION_DB_SAFEGUARD_NOTE}) ... a'b\\c\u{e9}"
    );
    assert_eq!(
        scrub_raw_model_text(&text, MODEL),
        "unknown-model ... (litellm_truncated skipped x chars. Truncation is a DB storage safeguard. Full, untruncated data is logged to logging callbacks (OTEL, Datadog, etc.). To increase the truncation limit, set `MAX_STRING_LENGTH_PROMPT_IN_DB` in your env.) ... unknown-model"
    );
}

#[test]
fn keeps_the_text_when_the_raw_model_is_empty() {
    assert_eq!(scrub_raw_model_text("model text", ""), "model text");
}

#[test]
fn touches_only_error_message_and_traceback() {
    let information = value(
        r#"{"error_message":"gpt-4o failed","traceback":"gpt-4o","error_class":"gpt-4o","error_code":400}"#,
    );
    let SpendValue::Mapping(mapping) = information else {
        panic!("error information is a mapping")
    };
    let scrubbed = scrub_raw_model_from_errors(&mapping, "gpt-4o");
    assert_eq!(
        lookup(&scrubbed, "error_message"),
        &Json::Text("unknown-model failed".to_owned())
    );
    assert_eq!(
        lookup(&scrubbed, "traceback"),
        &Json::Text("unknown-model".to_owned())
    );
    assert_eq!(
        lookup(&scrubbed, "error_class"),
        &Json::Text("gpt-4o".to_owned())
    );
}

#[test]
fn escapes_the_chosen_quote_for_repr_spellings() {
    assert_eq!(
        scrub_raw_model_text("it's a\"b and it\\'s a\"b", "it's a\"b"),
        "unknown-model and unknown-model"
    );
}
