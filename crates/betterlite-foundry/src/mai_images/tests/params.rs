use betterlite_prompts::arguments::json::Json;
use serde_json::Number;

use crate::mai_images::error::MaiImageError;
use crate::mai_images::params::SUPPORTED_OPENAI_PARAMS;

use super::support::{kind, mapped, object};

const DEFAULTS: &str = r#"{"width":1024,"height":1024}"#;

fn size_kind(size: &str, drop_params: bool) -> &'static str {
    kind(mapped(
        &format!(r#"{{"size":"{size}"}}"#),
        "{}",
        drop_params,
    ))
}

fn count_kind(count: &str, drop_params: bool) -> &'static str {
    kind(mapped(&format!(r#"{{"n":{count}}}"#), "{}", drop_params))
}

fn sized(size: &str) -> MaiImageError {
    mapped(&format!(r#"{{"size":"{size}"}}"#), "{}", false).unwrap_err()
}

pub(super) fn default_dimensions() {
    assert_eq!(mapped("{}", "{}", false).unwrap(), object(DEFAULTS));
    assert_eq!(
        mapped(r#"{"width":800,"height":5}"#, "{}", false).unwrap(),
        object(r#"{"width":800,"height":5}"#)
    );
    assert_eq!(
        mapped(r#"{"width":800}"#, "{}", false).unwrap(),
        object(r#"{"width":800,"height":1024}"#)
    );
    assert_eq!(
        mapped(r#"{"size":""}"#, "{}", false).unwrap(),
        object(DEFAULTS)
    );
    assert_eq!(
        mapped(r#"{"size":null}"#, "{}", false).unwrap(),
        object(DEFAULTS)
    );
    assert_eq!(kind(mapped(r#"{"size":1024}"#, "{}", false)), "field_type");
    existing_keys_win();
}

// a key that optional_params already has skips the matching non-default input
fn existing_keys_win() {
    let existing = mapped(r#"{"width":1,"height":2}"#, r#"{"width":2000}"#, false);
    assert_eq!(existing.unwrap(), object(r#"{"width":2000,"height":2}"#));
    let skipped = mapped(
        r#"{"size":"1024x1024","n":3}"#,
        r#"{"size":"huge","n":1}"#,
        false,
    );
    assert_eq!(
        skipped.unwrap(),
        object(r#"{"n":1,"width":1024,"height":1024}"#)
    );
}

pub(super) fn known_and_custom_size() {
    let accepted = |size: &str| mapped(&format!(r#"{{"size":"{size}"}}"#), "{}", false).unwrap();
    assert_eq!(accepted("1024x1024"), object(DEFAULTS));
    assert_eq!(
        accepted("1024x768"),
        object(r#"{"width":1024,"height":768}"#)
    );
    assert_eq!(
        accepted(" 800 x 1000 "),
        object(r#"{"width":800,"height":1000}"#)
    );
    assert_eq!(size_kind("1024X768", false), "unsupported_size");
    assert_eq!(size_kind("1024X1024", false), "unsupported_size");
    assert_eq!(size_kind("large", false), "unsupported_size");
    malformed_sizes();
    named_sizes_fail_limits();
}

fn malformed_sizes() {
    for malformed in [
        "axb",
        "1024x",
        "x768",
        "1024x768x1",
        "1024x768X",
        "1.5x1024",
    ] {
        assert_eq!(size_kind(malformed, false), "invalid_size", "{malformed}");
    }
    assert_eq!(size_kind("1024xq", true), "invalid_size");
    let invalid = sized("1024xq");
    assert_eq!(
        invalid.to_string(),
        "Invalid size format: '1024xq'. Expected format 'WIDTHxHEIGHT' (e.g., '1024x1024')."
    );
    assert_eq!(invalid.status_code(), Some(400));
    assert_eq!(invalid.llm_provider(), Some("azure_ai"));
    assert_eq!(
        sized("large").to_string(),
        "Unsupported size value: 'large'. Use a known size (e.g., '1024x1024') or a custom 'WIDTHxHEIGHT' string."
    );
}

// the named sizes are recognized, then fail the MAI limits
fn named_sizes_fail_limits() {
    assert_eq!(size_kind("512x512", false), "below_minimum");
    assert_eq!(size_kind("256x256", false), "below_minimum");
    assert_eq!(size_kind("1792x1024", false), "above_maximum");
    assert_eq!(size_kind("1024x1792", false), "above_maximum");
}

pub(super) fn dimension_and_pixel_limit() {
    assert_eq!(size_kind("768x768", false), "ok");
    assert_eq!(size_kind("768x1376", false), "ok");
    assert_eq!(size_kind("1024x1032", false), "ok");
    assert_eq!(size_kind("767x1024", false), "below_minimum");
    assert_eq!(size_kind("1024x767", false), "below_minimum");
    assert_eq!(size_kind("-5x2000", false), "below_minimum");
    assert_eq!(size_kind("768x1377", false), "above_maximum");
    assert_eq!(size_kind("99999999999x99999999999", false), "above_maximum");
    // the size checks run even when drop_params asks to drop what is unsupported
    assert_eq!(size_kind("512x512", true), "below_minimum");
    assert_eq!(size_kind("1792x1024", true), "above_maximum");
    limit_messages();
}

fn limit_messages() {
    assert_eq!(
        sized("768x1377").to_string(),
        "Unsupported size value: '768x1377'. Azure AI MAI image models accept at most 1056768 total pixels (768x1377 is 1057536)."
    );
    assert_eq!(
        sized("1792x1024").to_string(),
        "Unsupported size value: '1792x1024'. Azure AI MAI image models accept at most 1056768 total pixels (1792x1024 is 1835008)."
    );
    assert_eq!(
        sized("512x512").to_string(),
        "Unsupported size value: '512x512'. Azure AI MAI image models require width and height of at least 768 pixels."
    );
}

pub(super) fn n_one_only() {
    for kept in ["1", r#""1""#, "true", "null", r#"" 1 ""#] {
        let input = format!(r#"{{"n":{kept}}}"#);
        let optional = mapped(&input, "{}", false).unwrap();
        assert_eq!(optional.get("n"), object(&input).get("n"));
        assert_eq!(optional.len(), 3);
    }
    let skipped = mapped(r#"{"n":4}"#, r#"{"n":1}"#, false).unwrap();
    assert_eq!(skipped.get("n"), Some(&Json::Number(Number::from(1_i64))));
    counts_other_than_one();
    counts_that_are_not_whole();
}

fn counts_other_than_one() {
    for other in ["2", "0", "false", r#""3""#] {
        assert_eq!(count_kind(other, false), "count_not_one", "{other}");
        let input = format!(r#"{{"n":{other}}}"#);
        assert_eq!(mapped(&input, "{}", true).unwrap(), object(DEFAULTS));
    }
    let error = mapped(r#"{"n":2}"#, "{}", false).unwrap_err();
    assert_eq!(error.status_code(), Some(400));
    assert!(
        error
            .to_string()
            .starts_with("n=2 is not supported for model MAI-Image-2.5.")
    );
    let quoted = mapped(r#"{"n":"3"}"#, "{}", false).unwrap_err();
    assert!(quoted.to_string().starts_with("n=\"3\" is not supported"));
}

fn counts_that_are_not_whole() {
    for not_whole in [r#""two""#, "1.5", "[1]", r#"{"n":1}"#, r#""""#] {
        assert_eq!(
            count_kind(not_whole, true),
            "count_not_whole",
            "{not_whole}"
        );
        assert_eq!(
            count_kind(not_whole, false),
            "count_not_whole",
            "{not_whole}"
        );
    }
    let error = mapped(r#"{"n":"two"}"#, "{}", false).unwrap_err();
    assert_eq!(
        error.to_string(),
        "n=\"two\" is not a whole number of images for model MAI-Image-2.5."
    );
    assert_eq!(error.status_code(), Some(400));
}

pub(super) fn drop_unsupported_fields() {
    assert_eq!(SUPPORTED_OPENAI_PARAMS, ["n", "size"]);
    let error = mapped(r#"{"quality":"hd"}"#, "{}", false).unwrap_err();
    assert!(matches!(
        &error,
        MaiImageError::UnsupportedParam { name, model } if name == "quality" && model == "MAI-Image-2.5"
    ));
    assert_eq!(
        error.to_string(),
        "Parameter quality is not supported for model MAI-Image-2.5. Supported parameters are n, size and width/height. Set drop_params=True to drop unsupported parameters."
    );
    assert_eq!(error.status_code(), Some(400));
    assert_eq!(
        mapped(r#"{"quality":"hd"}"#, "{}", true).unwrap(),
        object(DEFAULTS)
    );
    let rejected = mapped(r#"{"quality":"hd","width":900}"#, r#"{"user":"u"}"#, false);
    assert_eq!(kind(rejected), "unsupported_param");
    let kept = mapped(
        r#"{"quality":"hd","width":900}"#,
        r#"{"quality":"kept","user":"u"}"#,
        false,
    );
    assert_eq!(
        kept.unwrap(),
        object(r#"{"quality":"kept","user":"u","width":900,"height":1024}"#)
    );
}
