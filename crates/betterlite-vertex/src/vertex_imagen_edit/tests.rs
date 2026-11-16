#![cfg(test)]

use betterlite_http::proxy::UpstreamResponse;
use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::{ImageParams, ImageResponse};
use http::header::{HeaderMap, HeaderValue};
use rstest::rstest;

use super::error::ImagenEditError;
use super::input::ImagenFileInput;
use super::options::{SUPPORTED_OPENAI_PARAMS, map_openai_params};
use super::request::transform_image_edit_request;
use super::response::transform_image_edit_response;
use super::target::predict_url;
use crate::credential_sources::getters::VertexParams;
use crate::vertex_image_identity::error::ImageIdentityError;
use crate::vertex_image_identity::settings::ImageIdentitySettings;
use crate::vertex_image_identity::target::ImageTarget;
use crate::vertex_imagen_generation::error::ImagenGenerationError;

mod input;

fn params(source: &str) -> ImageParams {
    serde_json::from_str(source).expect("params source")
}

fn json_value(source: &str) -> Json {
    serde_json::from_str(source).expect("json source")
}

fn payload(
    prompt: Option<&str>,
    images: &mut [Option<ImagenFileInput<'_>>],
    mask: Option<&mut ImagenFileInput<'_>>,
    mapped: &ImageParams,
    max_depth: i64,
) -> Json {
    let (text, files) = transform_image_edit_request(prompt, Some(images), mask, mapped, max_depth)
        .expect("the request builds");
    assert!(files.is_empty());
    json_value(&text)
}

fn identity(project: Option<&str>, location: Option<&str>) -> ImageIdentitySettings {
    let mut settings = ImageIdentitySettings::default();
    settings.environment.project = project.map(str::to_owned);
    settings.environment.location = location.map(str::to_owned);
    settings
}

const NOW: i64 = 1_700_000_000;

fn reply(status: u16, headers: HeaderMap, body: &str) -> UpstreamResponse {
    UpstreamResponse {
        status,
        headers,
        body: body.as_bytes().to_vec(),
    }
}

fn encoded(response: &ImageResponse) -> Json {
    serde_json::from_str(&serde_json::to_string(response).expect("the response encodes"))
        .expect("the response is JSON")
}

#[test]
fn only_n_size_and_mask_are_advertised() {
    assert_eq!(SUPPORTED_OPENAI_PARAMS, ["n", "size", "mask"]);
}

#[rstest]
#[case::positive(r#"{"n":2}"#, r#"{"sampleCount":2}"#)]
#[case::negative(r#"{"n":-1}"#, r#"{"sampleCount":-1}"#)]
#[case::zero(r#"{"n":0}"#, r#"{"sampleCount":0}"#)]
#[case::absent("{}", "{}")]
#[case::null(r#"{"n":null}"#, "{}")]
fn an_n_maps_to_sample_count(#[case] source: &str, #[case] expected: &str) {
    let mapped = map_openai_params(&params(source)).expect("an integer n maps");
    assert_eq!(Json::Object(mapped), json_value(expected));
}

#[rstest]
#[case::text(r#"{"n":"two"}"#)]
#[case::float(r#"{"n":2.5}"#)]
#[case::flag(r#"{"n":true}"#)]
fn a_non_integer_n_names_the_field(#[case] source: &str) {
    let error = map_openai_params(&params(source)).expect_err("n is an integer");
    assert!(matches!(
        error,
        ImagenEditError::FieldType {
            param: "n",
            expected: "an integer"
        }
    ));
}

#[rstest]
#[case::square("1024x1024", "1:1")]
#[case::wide("1792x1024", "16:9")]
#[case::tall("1024x1792", "9:16")]
#[case::landscape("1280x896", "4:3")]
#[case::portrait("896x1280", "3:4")]
#[case::unknown("999x999", "1:1")]
fn a_size_maps_to_its_aspect_ratio(#[case] size: &str, #[case] ratio: &str) {
    let mapped =
        map_openai_params(&params(&format!(r#"{{"size":"{size}"}}"#))).expect("a string size maps");
    assert_eq!(
        Json::Object(mapped),
        json_value(&format!(r#"{{"aspectRatio":"{ratio}"}}"#))
    );
}

#[test]
fn a_null_size_is_square_and_a_non_string_size_names_the_field() {
    let mapped = map_openai_params(&params(r#"{"size":null}"#)).expect("a null size maps");
    assert_eq!(Json::Object(mapped), json_value(r#"{"aspectRatio":"1:1"}"#));
    let error = map_openai_params(&params(r#"{"size":1792}"#)).expect_err("a number is not a size");
    assert!(matches!(
        error,
        ImagenEditError::FieldType {
            param: "size",
            expected: "a string"
        }
    ));
}

#[test]
fn an_optional_param_outside_the_supported_set_is_dropped() {
    let mapped = map_openai_params(&params(r#"{"quality":"hd","n":2}"#)).expect("n maps");
    assert_eq!(Json::Object(mapped), json_value(r#"{"sampleCount":2}"#));
}

#[test]
fn the_body_carries_the_inpainting_parameters_and_one_raw_reference() {
    let mut images = [Some(ImagenFileInput::Bytes(b"\x89PNG\r\n\x1a\n".to_vec()))];
    let (text, files) = transform_image_edit_request(
        Some("make it blue"),
        Some(&mut images),
        None,
        &params("{}"),
        100,
    )
    .expect("the request builds");
    assert!(files.is_empty());
    assert!(text.starts_with(r#"{"instances":[{"prompt":"make it blue","referenceImages":["#));
    assert_eq!(
        json_value(&text),
        json_value(
            r#"{"instances":[{"prompt":"make it blue","referenceImages":[{"referenceType":"REFERENCE_TYPE_RAW","referenceId":1,"referenceImage":{"bytesBase64Encoded":"iVBORw0KGgo="}}]}],"parameters":{"sampleCount":1,"editMode":"EDIT_MODE_INPAINT_INSERTION","editConfig":{"baseSteps":50},"guidanceScale":7.5,"seed":null}}"#
        )
    );
}

#[test]
fn the_mask_follows_the_raw_references_and_repeats_an_id() {
    let mut images = [
        Some(ImagenFileInput::Bytes(b"raw-one".to_vec())),
        None,
        Some(ImagenFileInput::Bytes(b"raw-two".to_vec())),
    ];
    let mut mask = ImagenFileInput::Bytes(b"mask-bytes".to_vec());
    let body = payload(
        Some("fill"),
        &mut images,
        Some(&mut mask),
        &params("{}"),
        100,
    );
    assert_eq!(
        body,
        json_value(
            r#"{"instances":[{"prompt":"fill","referenceImages":[{"referenceType":"REFERENCE_TYPE_RAW","referenceId":1,"referenceImage":{"bytesBase64Encoded":"cmF3LW9uZQ=="}},{"referenceType":"REFERENCE_TYPE_RAW","referenceId":3,"referenceImage":{"bytesBase64Encoded":"cmF3LXR3bw=="}},{"referenceType":"REFERENCE_TYPE_MASK","referenceId":3,"referenceImage":{"bytesBase64Encoded":"bWFzay1ieXRlcw=="},"maskImageConfig":{"maskMode":"MASK_MODE_USER_PROVIDED","dilation":0.03}}]}],"parameters":{"sampleCount":1,"editMode":"EDIT_MODE_INPAINT_INSERTION","editConfig":{"baseSteps":50},"guidanceScale":7.5,"seed":null}}"#
        )
    );
}

#[test]
fn a_mask_alone_follows_an_all_none_image_list() {
    let mut images = [None, None];
    let mut mask = ImagenFileInput::Bytes(b"mask-bytes".to_vec());
    let body = payload(
        Some("fill"),
        &mut images,
        Some(&mut mask),
        &params("{}"),
        100,
    );
    assert_eq!(
        body,
        json_value(
            r#"{"instances":[{"prompt":"fill","referenceImages":[{"referenceType":"REFERENCE_TYPE_MASK","referenceId":1,"referenceImage":{"bytesBase64Encoded":"bWFzay1ieXRlcw=="},"maskImageConfig":{"maskMode":"MASK_MODE_USER_PROVIDED","dilation":0.03}}]}],"parameters":{"sampleCount":1,"editMode":"EDIT_MODE_INPAINT_INSERTION","editConfig":{"baseSteps":50},"guidanceScale":7.5,"seed":null}}"#
        )
    );
}

#[test]
fn an_absent_image_is_refused_before_the_mask_is_read() {
    let mut mask = ImagenFileInput::Text("never read".to_owned());
    let error =
        transform_image_edit_request(Some("fill"), None, Some(&mut mask), &params("{}"), 100)
            .expect_err("an absent image is no request");
    assert!(matches!(error, ImagenEditError::MissingImage));
    assert!(error.upstream_answer().is_none());
}

#[test]
fn an_empty_image_list_sends_the_mask_alone() {
    let mut images: Vec<Option<ImagenFileInput<'_>>> = Vec::new();
    let mut mask = ImagenFileInput::Bytes(b"mask-bytes".to_vec());
    let body = payload(
        Some("fill"),
        images.as_mut_slice(),
        Some(&mut mask),
        &params("{}"),
        100,
    );
    assert_eq!(
        body,
        json_value(
            r#"{"instances":[{"prompt":"fill","referenceImages":[{"referenceType":"REFERENCE_TYPE_MASK","referenceId":1,"referenceImage":{"bytesBase64Encoded":"bWFzay1ieXRlcw=="},"maskImageConfig":{"maskMode":"MASK_MODE_USER_PROVIDED","dilation":0.03}}]}],"parameters":{"sampleCount":1,"editMode":"EDIT_MODE_INPAINT_INSERTION","editConfig":{"baseSteps":50},"guidanceScale":7.5,"seed":null}}"#
        )
    );
}

#[test]
fn an_empty_image_list_without_a_mask_is_refused() {
    let mut images: Vec<Option<ImagenFileInput<'_>>> = Vec::new();
    let error = transform_image_edit_request(
        Some("fill"),
        Some(images.as_mut_slice()),
        None,
        &params("{}"),
        100,
    )
    .expect_err("no reference image");
    assert!(matches!(error, ImagenEditError::MissingImage));
}

#[test]
fn an_all_none_image_list_without_a_mask_is_refused() {
    let mut images = [None, None];
    let error =
        transform_image_edit_request(Some("fill"), Some(&mut images), None, &params("{}"), 100)
            .expect_err("no reference image");
    assert!(matches!(error, ImagenEditError::MissingImage));
}

#[test]
fn a_missing_prompt_is_refused_after_the_references_are_read() {
    let mut images = [Some(ImagenFileInput::Bytes(b"raw-one".to_vec()))];
    let error = transform_image_edit_request(None, Some(&mut images), None, &params("{}"), 100)
        .expect_err("a prompt is required");
    assert!(matches!(error, ImagenEditError::MissingPrompt));
    let mut broken = [Some(ImagenFileInput::Text("raw-one".to_owned()))];
    let read_error =
        transform_image_edit_request(None, Some(&mut broken), None, &params("{}"), 100)
            .expect_err("the image is read first");
    assert!(matches!(read_error, ImagenEditError::PlainString));
}

#[test]
fn an_empty_prompt_is_kept() {
    let mut images = [Some(ImagenFileInput::Bytes(b"\x89PNG\r\n\x1a\n".to_vec()))];
    let body = payload(Some(""), &mut images, None, &params("{}"), 100);
    assert_eq!(
        body,
        json_value(
            r#"{"instances":[{"prompt":"","referenceImages":[{"referenceType":"REFERENCE_TYPE_RAW","referenceId":1,"referenceImage":{"bytesBase64Encoded":"iVBORw0KGgo="}}]}],"parameters":{"sampleCount":1,"editMode":"EDIT_MODE_INPAINT_INSERTION","editConfig":{"baseSteps":50},"guidanceScale":7.5,"seed":null}}"#
        )
    );
}

#[test]
fn a_mapped_sample_count_reaches_the_parameters() {
    let mut images = [Some(ImagenFileInput::Bytes(b"abc".to_vec()))];
    let mapped = map_openai_params(&params(r#"{"n":3}"#)).expect("an integer n maps");
    let body = payload(Some("fill"), &mut images, None, &mapped, 100);
    assert_eq!(
        body,
        json_value(
            r#"{"instances":[{"prompt":"fill","referenceImages":[{"referenceType":"REFERENCE_TYPE_RAW","referenceId":1,"referenceImage":{"bytesBase64Encoded":"YWJj"}}]}],"parameters":{"sampleCount":3,"editMode":"EDIT_MODE_INPAINT_INSERTION","editConfig":{"baseSteps":50},"guidanceScale":7.5,"seed":null}}"#
        )
    );
}

#[test]
fn an_absent_or_null_sample_count_takes_one_sample() {
    let mut images = [Some(ImagenFileInput::Bytes(b"abc".to_vec()))];
    let absent = payload(Some("fill"), &mut images, None, &params("{}"), 100);
    let mut null_images = [Some(ImagenFileInput::Bytes(b"abc".to_vec()))];
    let null = payload(
        Some("fill"),
        &mut null_images,
        None,
        &params(r#"{"sampleCount":null}"#),
        100,
    );
    assert_eq!(absent, null);
    assert_eq!(
        absent,
        json_value(
            r#"{"instances":[{"prompt":"fill","referenceImages":[{"referenceType":"REFERENCE_TYPE_RAW","referenceId":1,"referenceImage":{"bytesBase64Encoded":"YWJj"}}]}],"parameters":{"sampleCount":1,"editMode":"EDIT_MODE_INPAINT_INSERTION","editConfig":{"baseSteps":50},"guidanceScale":7.5,"seed":null}}"#
        )
    );
}

#[test]
fn a_non_numeric_sample_count_names_the_field() {
    let mut images = [Some(ImagenFileInput::Bytes(b"abc".to_vec()))];
    let error = transform_image_edit_request(
        Some("fill"),
        Some(&mut images),
        None,
        &params(r#"{"sampleCount":"3"}"#),
        100,
    )
    .expect_err("a text sample count is no number");
    assert!(matches!(
        error,
        ImagenEditError::FieldType {
            param: "sampleCount",
            expected: "a number"
        }
    ));
}

#[test]
fn the_mapped_aspect_ratio_never_reaches_the_body() {
    let mut images = [Some(ImagenFileInput::Bytes(b"abc".to_vec()))];
    let (text, _files) = transform_image_edit_request(
        Some("fill"),
        Some(&mut images),
        None,
        &params(r#"{"aspectRatio":"16:9"}"#),
        100,
    )
    .expect("the request builds");
    assert!(!text.contains("aspectRatio"));
}

#[test]
fn the_image_and_the_mask_are_read_at_the_top_level() {
    let mut images = [Some(ImagenFileInput::Bytes(b"abc".to_vec()))];
    let mut mask = ImagenFileInput::Bytes(b"mask-bytes".to_vec());
    let body = payload(Some("fill"), &mut images, Some(&mut mask), &params("{}"), 0);
    assert_eq!(
        body,
        json_value(
            r#"{"instances":[{"prompt":"fill","referenceImages":[{"referenceType":"REFERENCE_TYPE_RAW","referenceId":1,"referenceImage":{"bytesBase64Encoded":"YWJj"}},{"referenceType":"REFERENCE_TYPE_MASK","referenceId":2,"referenceImage":{"bytesBase64Encoded":"bWFzay1ieXRlcw=="},"maskImageConfig":{"maskMode":"MASK_MODE_USER_PROVIDED","dilation":0.03}}]}],"parameters":{"sampleCount":1,"editMode":"EDIT_MODE_INPAINT_INSERTION","editConfig":{"baseSteps":50},"guidanceScale":7.5,"seed":null}}"#
        )
    );
}

#[test]
fn the_predict_path_hangs_off_the_regional_host() {
    let settings = identity(Some("my-project"), Some("us-central1"));
    let target = ImageTarget {
        model: "imagen-3.0-capability-001",
        api_base: None,
        params: VertexParams::default(),
    };
    assert_eq!(
        predict_url(&settings, &target).expect("the URL builds"),
        "https://us-central1-aiplatform.googleapis.com/v1/projects/my-project/locations/us-central1/publishers/google/models/imagen-3.0-capability-001:predict"
    );
}

#[test]
fn a_custom_base_replaces_the_host_and_keeps_the_resource_path() {
    let settings = identity(Some("my-project"), Some("europe-west4"));
    let target = ImageTarget {
        model: "imagen-3.0-capability-001",
        api_base: Some("https://proxy.example/"),
        params: VertexParams::default(),
    };
    assert_eq!(
        predict_url(&settings, &target).expect("the URL builds"),
        "https://proxy.example/v1/projects/my-project/locations/europe-west4/publishers/google/models/imagen-3.0-capability-001:predict"
    );
}

#[test]
fn an_empty_base_falls_back_to_the_regional_host() {
    let settings = identity(Some("my-project"), Some("us-central1"));
    let target = ImageTarget {
        model: "imagen-3.0-capability-001",
        api_base: Some(""),
        params: VertexParams::default(),
    };
    assert_eq!(
        predict_url(&settings, &target).expect("the URL builds"),
        "https://us-central1-aiplatform.googleapis.com/v1/projects/my-project/locations/us-central1/publishers/google/models/imagen-3.0-capability-001:predict"
    );
}

#[test]
fn a_vertex_ai_prefix_is_stripped_from_the_model() {
    let settings = identity(Some("my-project"), Some("us-central1"));
    let target = ImageTarget {
        model: "vertex_ai/imagen-3.0-capability-001",
        api_base: None,
        params: VertexParams::default(),
    };
    assert_eq!(
        predict_url(&settings, &target).expect("the URL builds"),
        "https://us-central1-aiplatform.googleapis.com/v1/projects/my-project/locations/us-central1/publishers/google/models/imagen-3.0-capability-001:predict"
    );
}

#[test]
fn the_project_and_location_are_required_even_behind_a_custom_base() {
    let target = ImageTarget {
        model: "imagen-3.0-capability-001",
        api_base: Some("https://proxy.example"),
        params: VertexParams::default(),
    };
    let no_project = identity(None, Some("us-central1"));
    assert!(matches!(
        predict_url(&no_project, &target),
        Err(ImagenEditError::Target(ImageIdentityError::MissingTarget))
    ));
    let no_location = identity(Some("my-project"), None);
    assert!(matches!(
        predict_url(&no_location, &target),
        Err(ImagenEditError::Target(ImageIdentityError::MissingTarget))
    ));
}

#[test]
fn a_location_the_host_builder_refuses_is_a_target_error() {
    let settings = identity(Some("my-project"), Some("bad/location"));
    let target = ImageTarget {
        model: "imagen-3.0-capability-001",
        api_base: None,
        params: VertexParams::default(),
    };
    let error = predict_url(&settings, &target).expect_err("the location is refused");
    assert!(matches!(
        error,
        ImagenEditError::Target(ImageIdentityError::Location(_))
    ));
    assert!(error.upstream_answer().is_none());
}

#[test]
fn only_predictions_holding_the_bytes_key_become_images() {
    let body = r#"{"predictions":[{"bytesBase64Encoded":"AAA","prompt":"ignored"},{"raiFilteredReason":"blocked"},{"bytesBase64Encoded":null}]}"#;
    let response = transform_image_edit_response(&reply(200, HeaderMap::new(), body), NOW)
        .expect("the reply decodes");
    assert_eq!(
        encoded(&response),
        json_value(
            r#"{"created":1700000000,"data":[{"b64_json":"AAA"},{}],"usage":{"input_tokens":0,"input_tokens_details":{"image_tokens":0,"text_tokens":0},"output_tokens":0,"total_tokens":0}}"#
        )
    );
}

#[test]
fn a_reply_without_predictions_is_an_empty_data_list() {
    let response = transform_image_edit_response(&reply(200, HeaderMap::new(), "{}"), NOW)
        .expect("the reply decodes");
    assert_eq!(
        encoded(&response),
        json_value(
            r#"{"created":1700000000,"data":[],"usage":{"input_tokens":0,"input_tokens_details":{"image_tokens":0,"text_tokens":0},"output_tokens":0,"total_tokens":0}}"#
        )
    );
}

#[test]
fn a_non_json_reply_keeps_the_upstream_answer() {
    let mut headers = HeaderMap::new();
    headers.insert("x-request-id", HeaderValue::from_static("req-1"));
    let error = transform_image_edit_response(&reply(502, headers, r#"{"predictions": ["#), NOW)
        .expect_err("a truncated reply fails");
    let (status, kept) = error
        .upstream_answer()
        .expect("a parse failure keeps the reply answer");
    assert_eq!(status, 502);
    assert_eq!(
        kept.get("x-request-id"),
        Some(&HeaderValue::from_static("req-1"))
    );
    assert!(matches!(
        error,
        ImagenEditError::Reply(ImagenGenerationError::NotJson { .. })
    ));
}

#[test]
fn a_predictions_value_that_is_not_a_list_is_a_shape_error() {
    let error =
        transform_image_edit_response(&reply(200, HeaderMap::new(), r#"{"predictions":5}"#), NOW)
            .expect_err("a number is not a list");
    assert!(matches!(
        error,
        ImagenEditError::Reply(ImagenGenerationError::Shape { .. })
    ));
    assert!(error.upstream_answer().is_none());
}
