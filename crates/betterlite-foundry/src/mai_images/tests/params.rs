use betterlite_prompts::arguments::json::Json;
use serde_json::Number;

use crate::mai_images::error::MaiImageError;
use crate::mai_images::params::SUPPORTED_OPENAI_PARAMS;

use super::support::{mapped, object};

const DEFAULTS: &str = r#"{"width":1024,"height":1024}"#;

fn rejection(non_default: &str, drop_params: bool) -> MaiImageError {
    mapped(non_default, "{}", drop_params).unwrap_err()
}

fn sized(size: &str, drop_params: bool) -> Result<(), MaiImageError> {
    mapped(&format!(r#"{{"size":"{size}"}}"#), "{}", drop_params).map(drop)
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
    // a key that optional_params already has skips the matching non-default input
    assert_eq!(
        mapped(r#"{"width":1,"height":2}"#, r#"{"width":2000}"#, false).unwrap(),
        object(r#"{"width":2000,"height":2}"#)
    );
    assert_eq!(
        mapped(
            r#"{"size":"1024x1024","n":3}"#,
            r#"{"size":"huge","n":1}"#,
            false
        )
        .unwrap(),
        object(r#"{"n":1,"width":1024,"height":1024}"#)
    );
    assert_eq!(
        mapped(r#"{"size":""}"#, "{}", false).unwrap(),
        object(DEFAULTS)
    );
    assert_eq!(
        mapped(r#"{"size":null}"#, "{}", false).unwrap(),
        object(DEFAULTS)
    );
    assert!(matches!(
        rejection(r#"{"size":1024}"#, false),
        MaiImageError::FieldType { field: "size" }
    ));
}

pub(super) fn known_and_custom_size() {
    let sized_params = |size| mapped(&format!(r#"{{"size":"{size}"}}"#), "{}", false);
    assert_eq!(sized_params("1024x1024").unwrap(), object(DEFAULTS));
    assert_eq!(
        sized_params("1024x768").unwrap(),
        object(r#"{"width":1024,"height":768}"#)
    );
    assert_eq!(
        sized_params(" 800 x 1000 ").unwrap(),
        object(r#"{"width":800,"height":1000}"#)
    );
    assert!(matches!(
        sized("1024X768", false).unwrap_err(),
        MaiImageError::UnsupportedSize { .. }
    ));
    assert_eq!(
        sized("large", false).unwrap_err().to_string(),
        "Unsupported size value: 'large'. Use a known size (e.g., '1024x1024') or a custom 'WIDTHxHEIGHT' string."
    );
    assert!(matches!(
        sized("axb", false).unwrap_err(),
        MaiImageError::InvalidSize { .. }
    ));
    assert!(matches!(
        sized("1024x", false).unwrap_err(),
        MaiImageError::InvalidSize { .. }
    ));
    assert!(matches!(
        sized("x768", false).unwrap_err(),
        MaiImageError::InvalidSize { .. }
    ));
    assert!(matches!(
        sized("1024x768x1", false).unwrap_err(),
        MaiImageError::InvalidSize { .. }
    ));
    assert!(matches!(
        sized("1024x768X", false).unwrap_err(),
        MaiImageError::InvalidSize { .. }
    ));
    assert!(matches!(
        sized("1.5x1024", false).unwrap_err(),
        MaiImageError::InvalidSize { .. }
    ));
    let invalid = sized("1024xq", false).unwrap_err();
    assert_eq!(
        invalid.to_string(),
        "Invalid size format: '1024xq'. Expected format 'WIDTHxHEIGHT' (e.g., '1024x1024')."
    );
    assert_eq!(invalid.status_code(), Some(400));
    // the named sizes are recognized, then fail the MAI limits
    assert!(matches!(
        sized("512x512", false).unwrap_err(),
        MaiImageError::SizeBelowMinimum { .. }
    ));
    assert!(matches!(
        sized("256x256", false).unwrap_err(),
        MaiImageError::SizeBelowMinimum { .. }
    ));
    assert!(matches!(
        sized("1792x1024", false).unwrap_err(),
        MaiImageError::SizeAboveMaximum { .. }
    ));
    assert!(matches!(
        sized("1024x1792", false).unwrap_err(),
        MaiImageError::SizeAboveMaximum { .. }
    ));
}

pub(super) fn dimension_and_pixel_limit() {
    sized("768x768", false).unwrap();
    sized("768x1376", false).unwrap();
    sized("1024x1032", false).unwrap();
    assert!(matches!(
        sized("767x1024", false).unwrap_err(),
        MaiImageError::SizeBelowMinimum { .. }
    ));
    assert!(matches!(
        sized("1024x767", false).unwrap_err(),
        MaiImageError::SizeBelowMinimum { .. }
    ));
    assert!(matches!(
        sized("-5x2000", false).unwrap_err(),
        MaiImageError::SizeBelowMinimum { .. }
    ));
    assert_eq!(
        sized("768x1377", false).unwrap_err().to_string(),
        "Unsupported size value: '768x1377'. Azure AI MAI image models accept at most 1056768 total pixels (768x1377 is 1057536)."
    );
    assert_eq!(
        sized("1792x1024", false).unwrap_err().to_string(),
        "Unsupported size value: '1792x1024'. Azure AI MAI image models accept at most 1056768 total pixels (1792x1024 is 1835008)."
    );
    assert_eq!(
        sized("512x512", false).unwrap_err().to_string(),
        "Unsupported size value: '512x512'. Azure AI MAI image models require width and height of at least 768 pixels."
    );
    // the size checks run even when drop_params asks to drop what is unsupported
    assert!(matches!(
        sized("512x512", true).unwrap_err(),
        MaiImageError::SizeBelowMinimum { .. }
    ));
    assert!(matches!(
        sized("1792x1024", true).unwrap_err(),
        MaiImageError::SizeAboveMaximum { .. }
    ));
    assert!(matches!(
        sized("99999999999x99999999999", false).unwrap_err(),
        MaiImageError::SizeAboveMaximum { .. }
    ));
}

fn kept_count(count: &str) {
    let optional = mapped(&format!(r#"{{"n":{count}}}"#), "{}", false).unwrap();
    assert_eq!(
        optional.get("n"),
        object(&format!(r#"{{"n":{count}}}"#)).get("n")
    );
    assert_eq!(optional.len(), 3);
}

fn dropped_count(count: &str, message_start: &str) {
    let input = format!(r#"{{"n":{count}}}"#);
    let error = rejection(&input, false);
    assert!(matches!(error, MaiImageError::CountNotOne { .. }));
    assert!(error.to_string().starts_with(message_start), "{error}");
    assert_eq!(error.status_code(), Some(400));
    assert_eq!(mapped(&input, "{}", true).unwrap(), object(DEFAULTS));
}

pub(super) fn n_one_only() {
    kept_count("1");
    kept_count(r#""1""#);
    kept_count("true");
    kept_count("null");
    kept_count(r#"" 1 ""#);
    dropped_count("2", "n=2 ");
    dropped_count("0", "n=0 ");
    dropped_count("false", "n=false ");
    dropped_count(r#""3""#, "n=\"3\" ");
    for not_whole in [r#""two""#, "1.5", "[1]", r#"{"n":1}"#, r#""""#] {
        let input = format!(r#"{{"n":{not_whole}}}"#);
        let error = rejection(&input, true);
        assert!(matches!(error, MaiImageError::CountNotWhole { .. }));
        assert_eq!(error.status_code(), Some(400));
    }
    assert_eq!(
        rejection(r#"{"n":"two"}"#, false).to_string(),
        "n=\"two\" is not a whole number of images for model MAI-Image-2.5."
    );
    let skipped = mapped(r#"{"n":4}"#, r#"{"n":1}"#, false).unwrap();
    assert_eq!(skipped.get("n"), Some(&Json::Number(Number::from(1_i64))));
}

pub(super) fn drop_unsupported_fields() {
    assert_eq!(SUPPORTED_OPENAI_PARAMS, ["n", "size"]);
    let error = rejection(r#"{"quality":"hd"}"#, false);
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
    assert!(matches!(
        mapped(r#"{"quality":"hd","width":900}"#, r#"{"user":"u"}"#, false).unwrap_err(),
        MaiImageError::UnsupportedParam { .. }
    ));
    assert_eq!(
        mapped(
            r#"{"quality":"hd","width":900}"#,
            r#"{"quality":"kept","user":"u"}"#,
            false
        )
        .unwrap(),
        object(r#"{"quality":"kept","user":"u","width":900,"height":1024}"#)
    );
}
