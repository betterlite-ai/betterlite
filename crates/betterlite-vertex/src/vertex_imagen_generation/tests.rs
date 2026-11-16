#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_models::decisions::{Extra, Json};
use betterlite_openai::api::image_contracts::records::{ImageData, ImageInputTokens, ImageUsage};
use http::header::{HeaderMap, HeaderValue};
use rstest::rstest;

use super::decode::decode_imagen_generation;
use super::error::ImagenGenerationError;
use super::options::{SUPPORTED_OPENAI_PARAMS, map_openai_params};
use super::request::transform_image_generation_request;
use super::response::{ImagenGenerationResponse, transform_image_generation_response};
use super::target::predict_url;
use crate::credential_sources::getters::VertexParams;
use crate::gemini_response::decode::MAX_RESPONSE_BYTES;
use crate::model_urls::error::VertexUrlError;
use crate::vertex_image_identity::error::ImageIdentityError;
use crate::vertex_image_identity::settings::{
    ImageEnvironment, ImageIdentitySettings, ImageInstance, ImageModule, ImageSecrets,
};
use crate::vertex_image_identity::target::ImageTarget;

const PREDICT_URL: &str = "https://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/publishers/google/models/imagen-3.0-generate-002:predict";

fn object(source: &str) -> Extra {
    serde_json::from_str(source).expect("object source")
}

fn json_value(source: &str) -> Json {
    serde_json::from_str(source).expect("json source")
}

fn wrapped(body: &Extra) -> Json {
    Json::Object(body.clone())
}

fn reply(body: &str) -> UpstreamResponse {
    UpstreamResponse {
        status: 200,
        headers: HeaderMap::new(),
        body: body.as_bytes().to_vec(),
    }
}

fn encoded(response: &ImagenGenerationResponse) -> String {
    serde_json::to_string(response).expect("the response encodes")
}

fn transformed(body: &str) -> ImagenGenerationResponse {
    let mut response = ImagenGenerationResponse::default();
    transform_image_generation_response(&reply(body), &mut response).expect("the reply transforms");
    response
}

fn target_settings() -> ImageIdentitySettings {
    ImageIdentitySettings {
        instance: ImageInstance::default(),
        environment: ImageEnvironment::default(),
        module: ImageModule {
            project: Some("proj".to_owned()),
            location: Some("us-central1".to_owned()),
            credentials: None,
        },
        secrets: ImageSecrets::default(),
    }
}

fn standard_target() -> ImageTarget<'static> {
    ImageTarget {
        model: "imagen-3.0-generate-002",
        api_base: None,
        params: VertexParams::default(),
    }
}

#[test]
fn the_supported_params_are_the_openai_pair() {
    assert_eq!(SUPPORTED_OPENAI_PARAMS, ["n", "size"]);
}

#[test]
fn n_becomes_the_sample_count() {
    let mapped = map_openai_params(&object(r#"{"n":3}"#), &object("{}")).expect("n maps");
    assert_eq!(wrapped(&mapped), json_value(r#"{"sampleCount":3}"#));
}

#[test]
fn a_given_optional_key_suppresses_its_mapping() {
    let mapped = map_openai_params(
        &object(r#"{"n":2,"size":"1792x1024"}"#),
        &object(r#"{"n":5,"size":"1024x1792"}"#),
    )
    .expect("the given keys win");
    assert_eq!(wrapped(&mapped), json_value("{}"));
}

#[rstest]
#[case("1024x1024", "1:1")]
#[case("1792x1024", "16:9")]
#[case("1024x1792", "9:16")]
#[case("1280x896", "4:3")]
#[case("896x1280", "3:4")]
fn each_named_size_maps_to_its_aspect_ratio(#[case] size: &str, #[case] ratio: &str) {
    let source = format!(r#"{{"size":"{size}"}}"#);
    let mapped = map_openai_params(&object(&source), &object("{}")).expect("a string size maps");
    let expected = format!(r#"{{"aspectRatio":"{ratio}"}}"#);
    assert_eq!(wrapped(&mapped), json_value(&expected));
}

#[rstest]
#[case("640x480")]
#[case("")]
fn an_unknown_size_asks_for_a_square(#[case] size: &str) {
    let source = format!(r#"{{"size":"{size}"}}"#);
    let mapped = map_openai_params(&object(&source), &object("{}")).expect("any size maps");
    assert_eq!(
        wrapped(&mapped),
        json_value(r#"{"aspectRatio":"1:1"}"#),
        "{size}"
    );
}

#[test]
fn a_size_that_is_not_a_string_names_the_field() {
    let error = map_openai_params(&object(r#"{"size":1024}"#), &object("{}"))
        .expect_err("a number is not a size");
    assert!(matches!(
        error,
        ImagenGenerationError::FieldType { param: "size", .. }
    ));
}

#[test]
fn keys_outside_the_pair_are_ignored() {
    let mapped = map_openai_params(&object(r#"{"quality":"hd"}"#), &object("{}"))
        .expect("an unsupported key maps to nothing");
    assert_eq!(wrapped(&mapped), json_value("{}"));
}

#[test]
fn the_body_is_one_prompt_and_the_default_sample_count() {
    let body = transform_image_generation_request("a cat", &object("{}"), None);
    assert_eq!(
        wrapped(&body),
        json_value(r#"{"instances":[{"prompt":"a cat"}],"parameters":{"sampleCount":1}}"#)
    );
}

#[test]
fn an_explicit_zero_sample_count_beats_the_default() {
    let body = transform_image_generation_request("p", &object(r#"{"sampleCount":0}"#), None);
    assert_eq!(
        wrapped(&body),
        json_value(r#"{"instances":[{"prompt":"p"}],"parameters":{"sampleCount":0}}"#)
    );
}

#[test]
fn labels_are_popped_before_the_parameters_merge() {
    let body = transform_image_generation_request(
        "p",
        &object(r#"{"labels":{"team":"a"},"sampleCount":2}"#),
        None,
    );
    assert_eq!(
        wrapped(&body),
        json_value(
            r#"{"instances":[{"prompt":"p"}],"parameters":{"sampleCount":2},"labels":{"team":"a"}}"#
        )
    );
}

#[test]
fn labels_fall_back_to_the_request_metadata() {
    let params = json_value(r#"{"metadata":{"requester_metadata":{"user":"max"}}}"#);
    let body = transform_image_generation_request("p", &object("{}"), Some(&params));
    assert_eq!(
        wrapped(&body),
        json_value(
            r#"{"instances":[{"prompt":"p"}],"parameters":{"sampleCount":1},"labels":{"user":"max"}}"#
        )
    );
}

#[test]
fn non_text_label_values_are_dropped() {
    let body = transform_image_generation_request(
        "p",
        &object(r#"{"labels":{"team":"a","count":2,"nested":{"x":1}}}"#),
        None,
    );
    assert_eq!(
        wrapped(&body),
        json_value(
            r#"{"instances":[{"prompt":"p"}],"parameters":{"sampleCount":1},"labels":{"team":"a"}}"#
        )
    );
}

#[test]
fn a_labels_value_that_is_not_an_object_is_dropped() {
    let body = transform_image_generation_request("p", &object(r#"{"labels":"team"}"#), None);
    assert_eq!(
        wrapped(&body),
        json_value(r#"{"instances":[{"prompt":"p"}],"parameters":{"sampleCount":1}}"#)
    );
}

#[test]
fn an_empty_labels_object_is_dropped() {
    let body = transform_image_generation_request("p", &object(r#"{"labels":{}}"#), None);
    assert_eq!(
        wrapped(&body),
        json_value(r#"{"instances":[{"prompt":"p"}],"parameters":{"sampleCount":1}}"#)
    );
}

#[test]
fn an_empty_prompt_still_makes_one_instance() {
    let body = transform_image_generation_request("", &object("{}"), None);
    assert_eq!(
        wrapped(&body),
        json_value(r#"{"instances":[{"prompt":""}],"parameters":{"sampleCount":1}}"#)
    );
}

#[test]
fn every_prediction_with_the_encoded_key_is_appended() {
    let response = transformed(
        r#"{"predictions":[{"bytesBase64Encoded":"AAA"},{"bytesBase64Encoded":"BBB"}]}"#,
    );
    assert_eq!(
        encoded(&response),
        r#"{"data":[{"b64_json":"AAA"},{"b64_json":"BBB"}]}"#
    );
}

#[test]
fn a_prediction_without_the_key_appends_nothing() {
    let response = transformed(
        r#"{"predictions":[{"raiFilteredReason":"blocked"},{"bytesBase64Encoded":"AAA"}]}"#,
    );
    assert_eq!(encoded(&response), r#"{"data":[{"b64_json":"AAA"}]}"#);
}

#[test]
fn an_empty_encoded_image_is_still_appended() {
    let response = transformed(r#"{"predictions":[{"bytesBase64Encoded":""}]}"#);
    assert_eq!(encoded(&response), r#"{"data":[{"b64_json":""}]}"#);
}

#[test]
fn a_null_encoded_image_appends_an_empty_image() {
    let response = transformed(r#"{"predictions":[{"bytesBase64Encoded":null}]}"#);
    assert_eq!(encoded(&response), r#"{"data":[{}]}"#);
}

#[test]
fn images_append_to_existing_data_and_keep_the_usage() {
    let usage = ImageUsage {
        input_tokens: 3,
        input_tokens_details: ImageInputTokens::default(),
        output_tokens: 1,
        total_tokens: 4,
    };
    let mut response = ImagenGenerationResponse {
        data: vec![ImageData {
            b64_json: Some("OLD".to_owned()),
            revised_prompt: None,
            url: None,
        }],
        usage: Some(usage.clone()),
    };
    transform_image_generation_response(
        &reply(r#"{"predictions":[{"bytesBase64Encoded":"NEW"}]}"#),
        &mut response,
    )
    .expect("the reply transforms");
    let encoded_images: Vec<_> = response
        .data
        .iter()
        .map(|image| image.b64_json.clone())
        .collect();
    assert_eq!(
        encoded_images,
        [Some("OLD".to_owned()), Some("NEW".to_owned())]
    );
    assert_eq!(response.usage, Some(usage));
}

#[rstest]
#[case("{}")]
#[case(r#"{"predictions":[]}"#)]
fn absent_or_empty_predictions_append_nothing(#[case] body: &str) {
    assert_eq!(
        transformed(body),
        ImagenGenerationResponse::default(),
        "{body}"
    );
}

#[test]
fn a_body_that_is_not_json_keeps_the_upstream_status_and_headers() {
    let mut reply = reply(r#"{"predictions": ["#);
    reply.status = 502;
    reply
        .headers
        .insert("x-request-id", HeaderValue::from_static("req-1"));
    let error = decode_imagen_generation(&reply).expect_err("truncated JSON cannot be a reply");
    assert!(matches!(error, ImagenGenerationError::NotJson { .. }));
    let (status, headers) = error.upstream_answer().expect("the reply's answer is kept");
    assert_eq!(status, 502);
    assert_eq!(
        headers
            .get("x-request-id")
            .and_then(|value| value.to_str().ok()),
        Some("req-1")
    );
}

#[test]
fn an_empty_body_keeps_the_upstream_status() {
    let mut reply = reply("");
    reply.status = 503;
    let error = decode_imagen_generation(&reply).expect_err("an empty body is not JSON");
    assert!(matches!(
        error,
        ImagenGenerationError::NotJson { status: 503, .. }
    ));
}

#[test]
fn trailing_bytes_after_the_document_keep_the_upstream_answer() {
    let mut reply = reply(r#"{"predictions":[]}x"#);
    reply.status = 500;
    let error = decode_imagen_generation(&reply).expect_err("trailing bytes are not JSON");
    assert!(
        matches!(&error, ImagenGenerationError::NotJson { status: 500, .. }),
        "{error}"
    );
}

#[rstest]
#[case("[]")]
#[case(r#"[{"bytesBase64Encoded":"AAA"}]"#)]
#[case(r#"[[{"bytesBase64Encoded":"AAA"}]]"#)]
#[case("null")]
#[case("5")]
#[case(r#""text""#)]
fn a_valid_document_that_is_not_an_object_names_the_root(#[case] body: &str) {
    let error = decode_imagen_generation(&reply(body)).expect_err("the root is not an object");
    assert!(
        matches!(&error, ImagenGenerationError::Shape { path, .. } if path == "the document root"),
        "{body}: {error}"
    );
    assert!(error.upstream_answer().is_none(), "{body}");
}

#[test]
fn a_null_predictions_value_names_its_path() {
    let error =
        decode_imagen_generation(&reply(r#"{"predictions":null}"#)).expect_err("null is no list");
    assert!(
        matches!(&error, ImagenGenerationError::Shape { path, .. } if path == "predictions"),
        "{error}"
    );
}

#[test]
fn a_truncated_body_with_a_wrong_typed_value_keeps_the_upstream_answer() {
    let mut reply = reply(r#"{"predictions":"x""#);
    reply.status = 503;
    reply
        .headers
        .insert("x-request-id", HeaderValue::from_static("req-2"));
    let error = decode_imagen_generation(&reply).expect_err("a truncated body is not JSON");
    assert!(
        matches!(&error, ImagenGenerationError::NotJson { status: 503, .. }),
        "{error}"
    );
    let (_, headers) = error.upstream_answer().expect("the reply's answer is kept");
    assert_eq!(
        headers
            .get("x-request-id")
            .and_then(|value| value.to_str().ok()),
        Some("req-2")
    );
}

#[test]
fn trailing_bytes_after_a_wrong_typed_value_keep_the_upstream_answer() {
    let mut reply = reply(r#"{"predictions":[{"bytesBase64Encoded":5}],"#);
    reply.status = 502;
    let error = decode_imagen_generation(&reply).expect_err("the document is cut short");
    assert!(
        matches!(&error, ImagenGenerationError::NotJson { status: 502, .. }),
        "{error}"
    );
}

#[rstest]
#[case(r#"{"predictions":[["AAA"]]}"#)]
#[case(r#"{"predictions":[[null]]}"#)]
#[case(r#"{"predictions":[[]]}"#)]
#[case(r#"{"predictions":[null]}"#)]
fn a_prediction_that_is_a_list_or_null_names_its_index(#[case] body: &str) {
    let error = decode_imagen_generation(&reply(body)).expect_err("only an object is a prediction");
    assert!(
        matches!(&error, ImagenGenerationError::Shape { path, .. } if path == "predictions[0]"),
        "{body}: {error}"
    );
    assert!(error.upstream_answer().is_none(), "{body}");
    assert!(error.to_string().contains("predictions[0]"), "{body}");
    let ImagenGenerationError::Shape { source, .. } = &error else {
        panic!("{body}: {error}");
    };
    assert!(source.to_string().contains("a JSON object"), "{body}");
}

#[test]
fn a_predictions_value_that_is_not_a_list_names_its_path() {
    let error = decode_imagen_generation(&reply(r#"{"predictions":"x"}"#))
        .expect_err("a string is no list");
    assert!(
        matches!(&error, ImagenGenerationError::Shape { path, .. } if path == "predictions"),
        "{error}"
    );
}

#[test]
fn a_prediction_that_is_not_an_object_names_its_index() {
    let error = decode_imagen_generation(&reply(r#"{"predictions":["x"]}"#))
        .expect_err("a string is no prediction");
    assert!(
        matches!(&error, ImagenGenerationError::Shape { path, .. } if path == "predictions[0]"),
        "{error}"
    );
}

#[test]
fn a_non_string_encoded_image_names_its_path() {
    let error = decode_imagen_generation(&reply(r#"{"predictions":[{"bytesBase64Encoded":5}]}"#))
        .expect_err("a number is no encoded image");
    assert!(
        matches!(&error, ImagenGenerationError::Shape { path, .. } if path == "predictions[0].bytesBase64Encoded"),
        "{error}"
    );
}

#[test]
fn leading_whitespace_before_the_object_still_parses() {
    let decoded = decode_imagen_generation(&reply("  {\"predictions\":[]}"))
        .expect("leading whitespace is not part of the document");
    assert!(decoded.predictions.is_empty());
}

#[test]
fn a_reply_at_the_limit_still_parses() {
    let mut reply = reply(r#"{"predictions":[]}"#);
    reply.body.resize(MAX_RESPONSE_BYTES, b' ');
    let decoded = decode_imagen_generation(&reply).expect("a body at the limit parses");
    assert!(decoded.predictions.is_empty());
}

#[test]
fn a_reply_past_the_limit_is_refused_before_it_parses() {
    let mut reply = reply("");
    reply.body = vec![b'{'; MAX_RESPONSE_BYTES + 1];
    let error = decode_imagen_generation(&reply).expect_err("the body is past the limit");
    assert!(matches!(
        error,
        ImagenGenerationError::TooLarge { size } if size == MAX_RESPONSE_BYTES + 1
    ));
}

#[test]
fn a_standard_target_predicts_on_the_model_path() {
    let url = predict_url(&target_settings(), &standard_target()).expect("the URL builds");
    assert_eq!(url, PREDICT_URL);
}

#[test]
fn the_vertex_ai_prefix_is_stripped_from_the_predict_url() {
    let target = ImageTarget {
        model: "vertex_ai/imagen-3.0-generate-002",
        api_base: None,
        params: VertexParams::default(),
    };
    let url = predict_url(&target_settings(), &target).expect("the URL builds");
    assert_eq!(url, PREDICT_URL);
}

#[test]
fn a_custom_base_is_returned_with_trailing_slashes_trimmed() {
    let target = ImageTarget {
        model: "imagen-3.0-generate-002",
        api_base: Some("https://proxy.example/predict///"),
        params: VertexParams::default(),
    };
    let url = predict_url(&ImageIdentitySettings::default(), &target)
        .expect("a custom base needs no project");
    assert_eq!(url, "https://proxy.example/predict");
}

#[test]
fn a_custom_base_that_names_the_gemini_method_is_swapped_to_predict() {
    let target = ImageTarget {
        model: "imagen-3.0-generate-002",
        api_base: Some("https://proxy.example/models/x:generateContent"),
        params: VertexParams::default(),
    };
    let url = predict_url(&ImageIdentitySettings::default(), &target)
        .expect("a custom base needs no project");
    assert_eq!(url, "https://proxy.example/models/x:predict");
}

#[test]
fn an_empty_custom_base_still_builds_the_standard_url() {
    let target = ImageTarget {
        model: "imagen-3.0-generate-002",
        api_base: Some(""),
        params: VertexParams::default(),
    };
    let url = predict_url(&target_settings(), &target).expect("the standard URL builds");
    assert_eq!(url, PREDICT_URL);
}

#[test]
fn a_missing_project_or_location_is_one_error() {
    let error = predict_url(&ImageIdentitySettings::default(), &standard_target())
        .expect_err("project and location are both missing");
    assert!(matches!(error, ImageIdentityError::MissingTarget));
    let mut project_only = target_settings();
    project_only.module.location = None;
    let error =
        predict_url(&project_only, &standard_target()).expect_err("the location is missing");
    assert!(matches!(error, ImageIdentityError::MissingTarget));
}

#[test]
fn a_location_outside_the_lowercase_host_alphabet_never_chooses_the_host() {
    let mut settings = target_settings();
    settings.module.location = Some("us-central1.evil.example".to_owned());
    let error = predict_url(&settings, &standard_target()).expect_err("the location is refused");
    assert!(matches!(
        error,
        ImageIdentityError::Location(VertexUrlError::InvalidLocation)
    ));
}

#[test]
fn the_global_location_uses_the_control_plane_host() {
    let mut settings = target_settings();
    settings.module.location = Some("global".to_owned());
    let url = predict_url(&settings, &standard_target()).expect("global is a valid location");
    assert_eq!(
        url,
        "https://aiplatform.googleapis.com/v1/projects/proj/locations/global/publishers/google/models/imagen-3.0-generate-002:predict"
    );
}

#[rstest]
fn replays_the_imagen_generation_corpus(#[files("fuzz/corpus/imagen_generation/*")] path: PathBuf) {
    let body = fs::read(path).expect("the corpus file is readable");
    let reply = UpstreamResponse {
        status: 200,
        headers: HeaderMap::new(),
        body,
    };
    drop(decode_imagen_generation(&reply));
}
