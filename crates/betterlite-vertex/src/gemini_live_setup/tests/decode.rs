#![cfg(test)]

use betterlite_prompts::arguments::json::Json;

use crate::gemini_live_setup::decode::decode_gemini_live_setup;
use crate::gemini_live_setup::error::LiveSetupError;

use super::support::{as_json, parse};

#[test]
fn a_setup_envelope_decodes_to_the_setup() {
    let setup = decode_gemini_live_setup(
        r#"{
            "setup": {
                "model": "models/x",
                "generationConfig": {"responseModalities": ["AUDIO"], "maxOutputTokens": 12},
                "realtimeInputConfig": {"automaticActivityDetection": {"disabled": true}}
            }
        }"#,
    )
    .expect("the setup decodes");
    assert_eq!(
        as_json(&setup),
        parse::<Json>(
            r#"{
                "model": "models/x",
                "generationConfig": {"responseModalities": ["AUDIO"], "maxOutputTokens": 12},
                "realtimeInputConfig": {"automaticActivityDetection": {"disabled": true}}
            }"#
        )
    );
}

#[test]
fn a_missing_or_null_setup_is_an_empty_setup() {
    assert_eq!(
        as_json(&decode_gemini_live_setup("{}").expect("the envelope decodes")),
        parse::<Json>("{}")
    );
    assert_eq!(
        as_json(&decode_gemini_live_setup(r#"{"setup": null}"#).expect("the envelope decodes")),
        parse::<Json>("{}")
    );
}

#[test]
fn invalid_json_names_no_field() {
    let error = decode_gemini_live_setup("{").expect_err("the json is invalid");
    assert!(matches!(&error, LiveSetupError::InvalidJson(_)));
}

#[test]
fn trailing_data_is_invalid() {
    let error =
        decode_gemini_live_setup(r#"{"setup": {}} {}"#).expect_err("trailing data is invalid");
    assert!(matches!(&error, LiveSetupError::InvalidJson(_)));
}

#[test]
fn a_wrongly_typed_field_names_its_path() {
    let error = decode_gemini_live_setup(
        r#"{"setup": {"generationConfig": {"responseModalities": "audio"}}}"#,
    )
    .expect_err("the modalities must be a list");
    assert!(
        matches!(&error, LiveSetupError::InvalidField { path, .. } if path.ends_with("responseModalities"))
    );
}

#[test]
fn a_fractional_token_limit_names_its_path() {
    let error =
        decode_gemini_live_setup(r#"{"setup": {"generationConfig": {"maxOutputTokens": 1.5}}}"#)
            .expect_err("the limit is an integer");
    assert!(
        matches!(&error, LiveSetupError::InvalidField { path, .. } if path.ends_with("maxOutputTokens"))
    );
}
