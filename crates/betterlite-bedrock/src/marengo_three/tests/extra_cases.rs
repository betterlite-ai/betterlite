use rstest::rstest;

use super::super::is_marengo_3_model;
use super::cases::{Expected, check};

const BUCKET_OWNER_MESSAGE: &str =
    "s3:// media requires the 'bucketOwner' parameter, the account id that owns the bucket";
const MULTI_INPUT_MESSAGE: &str =
    "Input type 'multi_input' requires a non-empty 'media_sources' mapping of name to media";

#[rstest]
#[case::inside_arn(
    Some("arn:aws:bedrock:us-east-1::foundation-model/twelvelabs.marengo-embed-3-0-v1:0"),
    true
)]
#[case::bare_marker(Some("marengo-embed-3-"), true)]
#[case::other_version(Some("twelvelabs.marengo-embed-2-7-v1:0"), false)]
#[case::wrong_case(Some("Marengo-Embed-3-0"), false)]
#[case::without_trailing_dash(Some("marengo-embed-3"), false)]
#[case::empty(Some(""), false)]
#[case::missing(None, false)]
fn model_marker(#[case] model: Option<&str>, #[case] expected: bool) {
    assert_eq!(is_marengo_3_model(model), expected);
}

#[rstest]
#[case::text_keeps_empty_input(
    r#"{"input": "", "params": {}}"#,
    Expected::Built(r#"{"inputType": "text", "text": {"inputText": ""}}"#)
)]
#[case::empty_owner_accepted(
    r#"{"input": "s3://bucket/key", "params": {"inputType": "image", "bucketOwner": ""}}"#,
    Expected::Built(
        r#"{"inputType": "image", "image": {"mediaSource": {"s3Location": {"uri": "s3://bucket/key", "bucketOwner": ""}}}}"#
    )
)]
#[case::video_s3_owner(
    r#"{"input": "s3://bucket/clip.mp4", "params": {"inputType": "video", "bucketOwner": "123456789012", "startSec": 1}}"#,
    Expected::Built(
        r#"{"inputType": "video", "video": {"mediaSource": {"s3Location": {"uri": "s3://bucket/clip.mp4", "bucketOwner": "123456789012"}}, "startSec": 1}}"#
    )
)]
#[case::audio_s3_owner(
    r#"{"input": "s3://bucket/song.wav", "params": {"inputType": "audio", "bucketOwner": "123456789012"}}"#,
    Expected::Built(
        r#"{"inputType": "audio", "audio": {"mediaSource": {"s3Location": {"uri": "s3://bucket/song.wav", "bucketOwner": "123456789012"}}}}"#
    )
)]
#[case::text_image_s3_owner(
    r#"{"input": "words", "params": {"inputType": "text_image", "media_source": "s3://bucket/pic.png", "bucketOwner": "123456789012"}}"#,
    Expected::Built(
        r#"{"inputType": "text_image", "text_image": {"inputText": "words", "mediaSource": {"s3Location": {"uri": "s3://bucket/pic.png", "bucketOwner": "123456789012"}}}}"#
    )
)]
#[case::multi_input_s3_owner(
    r#"{"input": "", "params": {"inputType": "multi_input", "media_sources": {"one": "s3://bucket/a.png"}, "bucketOwner": "123456789012"}}"#,
    Expected::Built(
        r#"{"inputType": "multi_input", "multi_input": {"mediaSources": [{"name": "one", "mediaType": "image", "s3Location": {"uri": "s3://bucket/a.png", "bucketOwner": "123456789012"}}]}}"#
    )
)]
#[case::null_owner_rejected(
    r#"{"input": "s3://bucket/key", "params": {"inputType": "image", "bucketOwner": null}}"#,
    Expected::Rejected(BUCKET_OWNER_MESSAGE)
)]
#[case::not_exact_scheme_is_inline(
    r#"{"input": "S3://bucket/key", "params": {"inputType": "image"}}"#,
    Expected::Built(
        r#"{"inputType": "image", "image": {"mediaSource": {"base64String": "S3://bucket/key"}}}"#
    )
)]
#[case::data_url_second_component(
    r#"{"input": "data:image/png;base64,QUJD,tail", "params": {"inputType": "image"}}"#,
    Expected::Built(
        r#"{"inputType": "image", "image": {"mediaSource": {"base64String": "QUJD"}}}"#
    )
)]
#[case::text_image_empty_media(
    r#"{"input": "words", "params": {"inputType": "text_image", "media_source": ""}}"#,
    Expected::Built(
        r#"{"inputType": "text_image", "text_image": {"inputText": "words", "mediaSource": {"base64String": ""}}}"#
    )
)]
#[case::text_image_s3_needs_owner(
    r#"{"input": "words", "params": {"inputType": "text_image", "media_source": "s3://b/k"}}"#,
    Expected::Rejected(BUCKET_OWNER_MESSAGE)
)]
#[case::multi_input_empty_text_omitted(
    r#"{"input": "", "params": {"inputType": "multi_input", "media_sources": {"one": "AAA"}}}"#,
    Expected::Built(
        r#"{"inputType": "multi_input", "multi_input": {"mediaSources": [{"name": "one", "mediaType": "image", "base64String": "AAA"}]}}"#
    )
)]
#[case::multi_input_missing_sources(
    r#"{"input": "x", "params": {"inputType": "multi_input"}}"#,
    Expected::Rejected(MULTI_INPUT_MESSAGE)
)]
#[case::multi_input_empty_sources(
    r#"{"input": "x", "params": {"inputType": "multi_input", "media_sources": {}}}"#,
    Expected::Rejected(MULTI_INPUT_MESSAGE)
)]
#[case::text_image_27_option_before_missing_media(
    r#"{"input": "x", "params": {"inputType": "text_image", "minClipSec": 1}}"#,
    Expected::Rejected(
        "Marengo 3.0 does not accept the Marengo 2.7 parameters minClipSec; set drop_params to drop them"
    )
)]
fn builds(#[case] call: &str, #[case] expected: Expected<'_>) {
    check(call, &expected);
}

#[rstest]
#[case::timed_fields_on_text(
    r#"{"input": "x", "params": {"embeddingScope": ["clip"], "startSec": 0, "segmentation": {"method": "dynamic", "dynamic": {"minDurationSec": 2}}}}"#,
    Expected::Rejected(
        "Input type 'text' does not accept startSec, segmentation, embeddingScope; set drop_params to drop them"
    )
)]
#[case::timed_fields_on_image(
    r#"{"input": "AAA", "params": {"input_type": "image", "endSec": 3}}"#,
    Expected::Rejected("Input type 'image' does not accept endSec; set drop_params to drop them")
)]
#[case::dropped_timed_fields_on_text(
    r#"{"input": "x", "drop_params": true, "params": {"startSec": 0, "embeddingType": ["fused_embedding"]}}"#,
    Expected::Built(r#"{"inputType": "text", "text": {"inputText": "x"}}"#)
)]
#[case::dropped_27_options(
    r#"{"input": "x", "drop_params": true, "params": {"textTruncate": "end", "lengthSec": 0, "useFixedLengthSec": false, "minClipSec": 2}}"#,
    Expected::Built(r#"{"inputType": "text", "text": {"inputText": "x"}}"#)
)]
#[case::all_four_27_options(
    r#"{"input": "x", "params": {"minClipSec": 2, "useFixedLengthSec": true, "lengthSec": 1, "textTruncate": "end"}}"#,
    Expected::Rejected(
        "Marengo 3.0 does not accept the Marengo 2.7 parameters textTruncate, lengthSec, useFixedLengthSec, minClipSec; set drop_params to drop them"
    )
)]
#[case::null_27_option_is_absent(
    r#"{"input": "x", "params": {"textTruncate": null}}"#,
    Expected::Built(r#"{"inputType": "text", "text": {"inputText": "x"}}"#)
)]
#[case::lax_numbers_from_strings_and_bools(
    r#"{"input": "AAA", "params": {"inputType": "video", "startSec": "1.5", "endSec": true, "segmentation": {"method": "fixed", "fixed": {"durationSec": "4"}}}}"#,
    Expected::Built(
        r#"{"inputType": "video", "video": {"mediaSource": {"base64String": "AAA"}, "startSec": 1.5, "endSec": 1, "segmentation": {"method": "fixed", "fixed": {"durationSec": 4}}}}"#
    )
)]
#[case::lax_whole_duration_text_and_bool(
    r#"{"input": "AAA", "params": {"inputType": "audio", "startSec": false, "segmentation": {"method": "dynamic", "dynamic": {"minDurationSec": " 4.0 "}}}}"#,
    Expected::Built(
        r#"{"inputType": "audio", "audio": {"mediaSource": {"base64String": "AAA"}, "startSec": 0, "segmentation": {"method": "dynamic", "dynamic": {"minDurationSec": 4}}}}"#
    )
)]
#[case::dynamic_segmentation(
    r#"{"input": "AAA", "params": {"inputType": "audio", "segmentation": {"method": "dynamic", "dynamic": {"minDurationSec": 3.0}}, "inferenceId": "id-1"}}"#,
    Expected::Built(
        r#"{"inferenceId": "id-1", "inputType": "audio", "audio": {"mediaSource": {"base64String": "AAA"}, "segmentation": {"method": "dynamic", "dynamic": {"minDurationSec": 3}}}}"#
    )
)]
fn policy(#[case] call: &str, #[case] expected: Expected<'_>) {
    check(call, &expected);
}

#[rstest]
#[case::bad_input_type(
    r#"{"input_type": "audio_only"}"#,
    "input_type: Input should be 'text', 'image', 'video', 'audio', 'text_image' or 'multi_input'"
)]
#[case::bad_media_source_type(
    r#"{"media_source": 5}"#,
    "media_source: Input should be a valid string"
)]
#[case::bad_media_sources_shape(
    r#"{"media_sources": ["a"]}"#,
    "media_sources: Input should be a valid dictionary"
)]
#[case::bad_media_sources_value(
    r#"{"media_sources": {"a": 1, "b": "ok", "c": null}}"#,
    "media_sources.a: Input should be a valid string; media_sources.c: Input should be a valid string"
)]
#[case::bad_times_in_order(
    r#"{"endSec": "x", "startSec": [1], "bucketOwner": 1}"#,
    "bucketOwner: Input should be a valid string; startSec: Input should be a valid number; endSec: Input should be a valid number"
)]
#[case::bad_numeric_text(
    r#"{"startSec": "", "endSec": "abc", "segmentation": {"method": "fixed", "fixed": {"durationSec": "1e2"}}}"#,
    "startSec: Input should be a valid number; endSec: Input should be a valid number; segmentation.TwelveLabsMarengo3FixedSegmentation.fixed.durationSec: Input should be a valid integer; segmentation.TwelveLabsMarengo3DynamicSegmentation.method: Input should be 'dynamic'; segmentation.TwelveLabsMarengo3DynamicSegmentation.dynamic: Field required"
)]
#[case::bad_option_items(
    r#"{"embeddingOption": ["visual", "bad", "worse"], "embeddingType": "fused_embedding", "embeddingScope": ["wide"]}"#,
    "embeddingOption.1: Input should be 'visual', 'audio' or 'transcription'; embeddingOption.2: Input should be 'visual', 'audio' or 'transcription'; embeddingType: Input should be a valid tuple; embeddingScope.0: Input should be 'clip' or 'asset'"
)]
#[case::segmentation_not_object(
    r#"{"segmentation": 3}"#,
    "segmentation.TwelveLabsMarengo3FixedSegmentation: Input should be a valid dictionary; segmentation.TwelveLabsMarengo3DynamicSegmentation: Input should be a valid dictionary"
)]
#[case::segmentation_fixed_bad_duration(
    r#"{"segmentation": {"method": "fixed", "fixed": {"durationSec": 1.5}}}"#,
    "segmentation.TwelveLabsMarengo3FixedSegmentation.fixed.durationSec: Input should be a valid integer, got a number with a fractional part; segmentation.TwelveLabsMarengo3DynamicSegmentation.method: Input should be 'dynamic'; segmentation.TwelveLabsMarengo3DynamicSegmentation.dynamic: Field required"
)]
#[case::segmentation_missing_method(
    r#"{"segmentation": {"dynamic": {"minDurationSec": 5}}}"#,
    "segmentation.TwelveLabsMarengo3FixedSegmentation.method: Field required; segmentation.TwelveLabsMarengo3FixedSegmentation.fixed: Field required; segmentation.TwelveLabsMarengo3DynamicSegmentation.method: Field required"
)]
#[case::bad_inference_id(r#"{"inferenceId": 7}"#, "inferenceId: Input should be a valid string")]
#[case::invalid_beats_27_options(
    r#"{"lengthSec": 3, "media_source": false}"#,
    "media_source: Input should be a valid string"
)]
fn invalid_parameters(#[case] params: &str, #[case] details: &str) {
    let call = format!(r#"{{"input": "x", "params": {params}}}"#);
    let message = format!("Invalid Marengo 3.0 parameters: {details}");
    check(&call, &Expected::Rejected(&message));
}
