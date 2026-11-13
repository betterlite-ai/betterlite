use rstest::rstest;
use serde::Deserialize;

use super::super::error::Marengo3Error;
use super::super::fields::Given;
use super::super::params::RawParams;
use super::super::{build_marengo_3_request, is_marengo_3_model};

#[derive(Deserialize)]
struct Call {
    input: String,
    params: RawParams,
    #[serde(default)]
    drop_params: bool,
}

pub(super) enum Expected<'text> {
    Built(&'text str),
    Rejected(&'text str),
}

pub(super) fn check(call: &str, expected: &Expected<'_>) {
    let parsed: Call = serde_json::from_str(call).unwrap();
    let outcome = build_marengo_3_request(&parsed.input, &parsed.params, parsed.drop_params);
    match (outcome, expected) {
        (Ok(request), Expected::Built(wanted)) => {
            let actual: Given =
                serde_json::from_str(&serde_json::to_string(&request).unwrap()).unwrap();
            let expected_field: Given = serde_json::from_str(wanted).unwrap();
            assert_eq!(actual, expected_field);
        }
        (Err(Marengo3Error::Provider { status, error }), Expected::Rejected(message)) => {
            assert_eq!(status, 400_u16);
            assert_eq!(error.error.message, *message);
        }
        (unexpected, _) => panic!("unexpected outcome {unexpected:?}"),
    }
}

#[rstest]
#[case::marengo3_model_detection(
    "amazon.marengo-embed-3-0-v1:0",
    true,
    r#"{"input": "clip", "params": {"inputType": "audio", "input_type": "video", "ignored": 1}}"#,
    Expected::Built(
        r#"{"inputType": "audio", "audio": {"mediaSource": {"base64String": "clip"}}}"#
    )
)]
#[case::named_media_sources(
    "twelvelabs.marengo-embed-3-0-v1:0",
    true,
    r#"{"input": "caption", "params": {
        "input_type": "multi_input",
        "bucketOwner": "123456789012",
        "inferenceId": "",
        "media_sources": {"b": "s3://bucket/key.png", "a": "data:image/png;base64,AAA"},
        "unknown": true
    }}"#,
    Expected::Built(
        r#"{
        "inferenceId": "",
        "inputType": "multi_input",
        "multi_input": {
            "inputText": "caption",
            "mediaSources": [
                {"name": "b", "mediaType": "image", "s3Location": {"uri": "s3://bucket/key.png", "bucketOwner": "123456789012"}},
                {"name": "a", "mediaType": "image", "base64String": "AAA"}
            ]
        }
    }"#
    )
)]
#[case::required_bucket_owner(
    "marengo-embed-3-0-v1:0",
    true,
    r#"{"input": "s3://bucket/key.png", "params": {"inputType": "image"}}"#,
    Expected::Rejected(
        "s3:// media requires the 'bucketOwner' parameter, the account id that owns the bucket"
    )
)]
#[case::text_versus_timed_options(
    "marengo-embed-3-0-v1:0",
    true,
    r#"{"input": "AAA", "params": {
        "inputType": "video",
        "startSec": 1,
        "endSec": 2.5,
        "segmentation": {"method": "fixed", "fixed": {"durationSec": 4, "extra": 1}, "ignored": 1},
        "embeddingOption": ["visual", "audio"],
        "embeddingType": ["fused_embedding"],
        "embeddingScope": ["clip", "asset"]
    }}"#,
    Expected::Built(
        r#"{
        "inputType": "video",
        "video": {
            "mediaSource": {"base64String": "AAA"},
            "startSec": 1,
            "endSec": 2.5,
            "segmentation": {"method": "fixed", "fixed": {"durationSec": 4}},
            "embeddingOption": ["visual", "audio"],
            "embeddingType": ["fused_embedding"],
            "embeddingScope": ["clip", "asset"]
        }
    }"#
    )
)]
#[case::reject_27_only_options(
    "marengo-embed-3-0-v1:0",
    true,
    r#"{"input": "hi", "params": {"minClipSec": "", "textTruncate": false, "lengthSec": 0, "startSec": 1}}"#,
    Expected::Rejected(
        "Marengo 3.0 does not accept the Marengo 2.7 parameters textTruncate, lengthSec, minClipSec; set drop_params to drop them"
    )
)]
#[case::drop_policy(
    "marengo-embed-3-0-v1:0",
    true,
    r#"{"input": "hi", "drop_params": true, "params": {"inputType": "bogus", "lengthSec": 5}}"#,
    Expected::Rejected(
        "Invalid Marengo 3.0 parameters: inputType: Input should be 'text', 'image', 'video', 'audio', 'text_image' or 'multi_input'"
    )
)]
#[case::require_input_media(
    "marengo-embed-3-0-v1:0",
    true,
    r#"{"input": "hi", "params": {"inputType": "text_image"}}"#,
    Expected::Rejected("Input type 'text_image' requires the 'media_source' parameter")
)]
fn marengo_three_cases(
    #[case] model: &str,
    #[case] detected: bool,
    #[case] call: &str,
    #[case] expected: Expected<'_>,
) {
    assert_eq!(is_marengo_3_model(Some(model)), detected);
    check(call, &expected);
}
