#![cfg(test)]

use betterlite_config::sections::vertex_credential_sources::VertexSettings;
use betterlite_config::sections::vertex_credential_sources::vertex_settings::AdcEnvironment;
use betterlite_openai::openai_video_identifiers::ids::{
    decode_video_id_with_provider, encode_video_id_with_provider,
};
use betterlite_openai::videos::reply::VideoObjectType;
use betterlite_prompts::arguments::json::{Json, Object};
use rstest::rstest;
use serde_json::Number;

use super::{VideoContext, create_response, status_request, status_response};
use crate::gemini_response::decode::MAX_RESPONSE_BYTES;
use crate::studio_veo_operation::decode::decode_studio_veo_operation;
use crate::studio_veo_operation::error::StudioVeoOperationError;
use crate::studio_veo_operation::unsupported::UnsupportedOperation;
use crate::vertex_veo_operation::decode::decoded_json;

const OPERATION: &str = "operations/generate_1234567890";
const MODEL: &str = "veo-3.0-generate-001";
const PROVIDER: &str = "gemini";
const STATUS_ID: &str = "video_bGl0ZWxsbTpjdXN0b21fbGxtX3Byb3ZpZGVyOmdlbWluaTttb2RlbF9pZDo7dmlkZW9faWQ6b3BlcmF0aW9ucy9nZW5lcmF0ZV8xMjM0NTY3ODkw";
const CREATE_ID: &str = "video_bGl0ZWxsbTpjdXN0b21fbGxtX3Byb3ZpZGVyOmdlbWluaTttb2RlbF9pZDp2ZW8tMy4wLWdlbmVyYXRlLTAwMTt2aWRlb19pZDpvcGVyYXRpb25zL2dlbmVyYXRlXzEyMzQ1Njc4OTA=";

fn object<const N: usize>(fields: [(&str, Json); N]) -> Object {
    fields
        .into_iter()
        .map(|(name, value)| (name.to_owned(), value))
        .collect()
}

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn number(value: i64) -> Json {
    Json::Number(Number::from(value))
}

fn decimal(value: f64) -> Json {
    Json::Number(Number::from_f64(value).expect("the decimal is finite"))
}

fn request_data(parameters: Object) -> Object {
    let mut data = Object::new();
    data.insert("parameters".to_owned(), Json::Object(parameters));
    data
}

fn settings(default_video_duration_seconds: i64) -> VertexSettings {
    VertexSettings {
        project: None,
        location: None,
        credentials: None,
        adc: AdcEnvironment::default(),
        default_video_duration_seconds,
    }
}

fn context<'call>(
    provider: Option<&'call str>,
    request_data: Option<&'call Object>,
    settings: &'call VertexSettings,
) -> VideoContext<'call> {
    VideoContext {
        provider,
        request_data,
        settings,
    }
}

fn encoded<T>(value: &T) -> Vec<u8>
where
    T: serde::Serialize,
{
    serde_json::to_vec(value).expect("the body serializes")
}

#[test]
fn create_returns_processing_with_the_model_and_usage() {
    let body = object([("name", text(OPERATION))]);
    let parameters = object([
        ("durationSeconds", number(6)),
        ("resolution", text(" 1080P ")),
        ("sampleCount", number(2)),
    ]);
    let data = request_data(parameters);
    let video = create_response(
        MODEL,
        &encoded(&body),
        &context(Some(PROVIDER), Some(&data), &settings(8)),
    )
    .expect("the processing response converts");
    assert_eq!(video.id, CREATE_ID);
    assert_eq!(
        video.id,
        encode_video_id_with_provider(OPERATION, PROVIDER, Some(MODEL))
    );
    assert_eq!(video.object, VideoObjectType::Video);
    assert_eq!(video.status, "processing");
    assert_eq!(video.model.as_deref(), Some(MODEL));
    assert_eq!(
        video.usage,
        Some(
            decoded_json(object([
                ("duration_seconds", decimal(6.0)),
                ("video_resolution", text("1080p")),
                ("video_count", number(2)),
            ]))
            .expect("the expected usage converts")
        )
    );
}

#[test]
fn create_leaves_every_optional_field_absent() {
    let body = object([("name", text(OPERATION))]);
    let video = create_response(MODEL, &encoded(&body), &context(None, None, &settings(8)))
        .expect("the processing response converts");
    assert_eq!(video.created_at, None);
    assert_eq!(video.completed_at, None);
    assert_eq!(video.expires_at, None);
    assert_eq!(video.error, None);
    assert_eq!(video.progress, None);
    assert_eq!(video.remixed_from_video_id, None);
    assert_eq!(video.seconds, None);
    assert_eq!(video.size, None);
    assert_eq!(
        video.usage,
        Some(decoded_json(Object::new()).expect("the empty usage converts"))
    );
}

#[test]
fn create_without_a_provider_keeps_the_operation_name() {
    let body = object([("name", text(OPERATION))]);
    let video = create_response(MODEL, &encoded(&body), &context(None, None, &settings(8)))
        .expect("the processing response converts");
    assert_eq!(video.id, OPERATION);
}

#[test]
fn an_empty_provider_keeps_the_operation_name() {
    let body = object([("name", text(OPERATION))]);
    let video = create_response(
        MODEL,
        &encoded(&body),
        &context(Some(""), None, &settings(8)),
    )
    .expect("the processing response converts");
    assert_eq!(video.id, OPERATION);
    let status = status_response(&encoded(&body), Some("")).expect("the operation converts");
    assert_eq!(status.id, OPERATION);
}

#[test]
fn an_empty_request_data_gives_an_empty_usage() {
    let body = object([("name", text(OPERATION))]);
    let video = create_response(
        MODEL,
        &encoded(&body),
        &context(Some(PROVIDER), Some(&Object::new()), &settings(8)),
    )
    .expect("the processing response converts");
    assert_eq!(
        video.usage,
        Some(decoded_json(Object::new()).expect("the empty usage converts"))
    );
}

#[test]
fn create_usage_defaults_the_duration() {
    let body = object([("name", text(OPERATION))]);
    let data = request_data(object([("resolution", text("720p"))]));
    let video = create_response(
        MODEL,
        &encoded(&body),
        &context(Some(PROVIDER), Some(&data), &settings(11)),
    )
    .expect("the processing response converts");
    assert_eq!(
        video.usage,
        Some(
            decoded_json(object([
                ("duration_seconds", decimal(11.0)),
                ("video_resolution", text("720p")),
            ]))
            .expect("the expected usage converts")
        )
    );
}

#[test]
fn a_whitespace_only_resolution_is_retained_as_an_empty_string() {
    let body = object([("name", text(OPERATION))]);
    let data = request_data(object([("resolution", text("   "))]));
    let video = create_response(
        MODEL,
        &encoded(&body),
        &context(Some(PROVIDER), Some(&data), &settings(8)),
    )
    .expect("the processing response converts");
    assert_eq!(
        video.usage,
        Some(
            decoded_json(object([
                ("duration_seconds", decimal(8.0)),
                ("video_resolution", text("")),
            ]))
            .expect("the expected usage converts")
        )
    );
}

#[rstest]
#[case::empty(Some(text("")))]
#[case::null(Some(Json::Null))]
#[case::absent(None)]
fn an_absent_resolution_is_omitted(#[case] resolution: Option<Json>) {
    let body = object([("name", text(OPERATION))]);
    let mut parameters = object([("durationSeconds", number(6))]);
    if let Some(value) = resolution {
        parameters.insert("resolution".to_owned(), value);
    }
    let data = request_data(parameters);
    let video = create_response(
        MODEL,
        &encoded(&body),
        &context(Some(PROVIDER), Some(&data), &settings(8)),
    )
    .expect("the processing response converts");
    assert_eq!(
        video.usage,
        Some(
            decoded_json(object([("duration_seconds", decimal(6.0))]))
                .expect("the expected usage converts")
        )
    );
}

#[test]
fn an_empty_operation_name_is_rejected() {
    let body = object([("name", text(""))]);
    assert!(matches!(
        create_response(MODEL, &encoded(&body), &context(None, None, &settings(8))),
        Err(StudioVeoOperationError::MissingName)
    ));
}

#[rstest]
#[case::absent(b"{}".as_slice())]
#[case::null(br#"{"name": null}"#.as_slice())]
#[case::number(br#"{"name": 5}"#.as_slice())]
fn an_unusable_operation_name_is_rejected(#[case] body: &[u8]) {
    assert!(matches!(
        create_response(MODEL, body, &context(None, None, &settings(8))),
        Err(StudioVeoOperationError::Json(_))
    ));
}

#[test]
fn a_non_object_parameters_block_is_rejected() {
    let body = object([("name", text(OPERATION))]);
    let data = object([("parameters", text("later"))]);
    assert!(matches!(
        create_response(
            MODEL,
            &encoded(&body),
            &context(Some(PROVIDER), Some(&data), &settings(8))
        ),
        Err(StudioVeoOperationError::Usage(_))
    ));
}

#[test]
fn a_done_operation_is_completed_without_a_model() {
    let body = object([("name", text(OPERATION)), ("done", Json::Bool(true))]);
    let video =
        status_response(&encoded(&body), Some(PROVIDER)).expect("the completed operation converts");
    assert_eq!(video.id, STATUS_ID);
    assert_eq!(
        video.id,
        encode_video_id_with_provider(OPERATION, PROVIDER, None)
    );
    assert_eq!(video.object, VideoObjectType::Video);
    assert_eq!(video.status, "completed");
    assert_eq!(video.model, None);
    assert_eq!(video.usage, None);
    assert_eq!(video.created_at, None);
    assert_eq!(video.error, None);
}

#[test]
fn an_unfinished_operation_is_processing() {
    let body = object([("name", text(OPERATION)), ("done", Json::Bool(false))]);
    let video = status_response(&encoded(&body), Some(PROVIDER))
        .expect("the processing operation converts");
    assert_eq!(video.status, "processing");
    assert_eq!(video.id, STATUS_ID);
}

#[test]
fn an_absent_done_field_is_false() {
    let body = object([("name", text(OPERATION))]);
    let video = status_response(&encoded(&body), None).expect("the sparse operation converts");
    assert_eq!(video.status, "processing");
}

#[test]
fn a_null_done_takes_the_false_default() {
    let body = object([("name", text(OPERATION)), ("done", Json::Null)]);
    let video = status_response(&encoded(&body), None).expect("the null done converts");
    assert_eq!(video.status, "processing");
}

#[test]
fn a_done_operation_with_an_error_is_completed() {
    let body = object([
        ("name", text(OPERATION)),
        ("done", Json::Bool(true)),
        (
            "error",
            Json::Object(object([("code", number(13)), ("message", text("boom"))])),
        ),
    ]);
    let video =
        status_response(&encoded(&body), Some(PROVIDER)).expect("the failed operation converts");
    assert_eq!(video.status, "completed");
    assert_eq!(video.error, None);
}

#[test]
fn status_keeps_the_operation_metadata_out_of_the_result() {
    let body = object([
        ("name", text(OPERATION)),
        ("done", Json::Bool(true)),
        (
            "metadata",
            Json::Object(object([
                ("createTime", text("2026-01-01T00:00:00.000000Z")),
                ("model", text(MODEL)),
            ])),
        ),
    ]);
    let video = status_response(&encoded(&body), None).expect("the operation converts");
    assert_eq!(video.created_at, None);
    assert_eq!(video.completed_at, None);
    assert_eq!(video.model, None);
}

#[test]
fn status_without_a_provider_keeps_the_operation_name() {
    let body = object([("name", text(OPERATION)), ("done", Json::Bool(true))]);
    let video = status_response(&encoded(&body), None).expect("the operation converts");
    assert_eq!(video.id, OPERATION);
}

#[rstest]
#[case::error_text(br#"{"name":"operations/o","error":"boom"}"#.as_slice())]
#[case::done_text(br#"{"name":"operations/o","done":"yes"}"#.as_slice())]
#[case::metadata_text(br#"{"name":"operations/o","metadata":"later"}"#.as_slice())]
#[case::response_text(br#"{"name":"operations/o","response":"later"}"#.as_slice())]
#[case::no_generate_video_response(br#"{"name":"operations/o","response":{}}"#.as_slice())]
#[case::samples_null(br#"{"name":"operations/o","response":{"generateVideoResponse":{"generatedSamples":null}}}"#.as_slice())]
#[case::samples_text(br#"{"name":"operations/o","response":{"generateVideoResponse":{"generatedSamples":"later"}}}"#.as_slice())]
#[case::sample_not_an_object(br#"{"name":"operations/o","response":{"generateVideoResponse":{"generatedSamples":[5]}}}"#.as_slice())]
#[case::video_absent(br#"{"name":"operations/o","response":{"generateVideoResponse":{"generatedSamples":[{}]}}}"#.as_slice())]
#[case::uri_number(br#"{"name":"operations/o","response":{"generateVideoResponse":{"generatedSamples":[{"video":{"uri":5}}]}}}"#.as_slice())]
fn malformed_operation_fields_are_rejected(#[case] body: &[u8]) {
    assert!(matches!(
        status_response(body, None),
        Err(StudioVeoOperationError::Json(_))
    ));
}

#[test]
fn a_completed_operation_keeps_its_samples() {
    let body = object([
        ("name", text(OPERATION)),
        ("done", Json::Bool(true)),
        (
            "response",
            Json::Object(object([(
                "generateVideoResponse",
                Json::Object(object([(
                    "generatedSamples",
                    Json::List(vec![Json::Object(object([(
                        "video",
                        Json::Object(object([("uri", text("files/abc:download"))])),
                    )]))]),
                )])),
            )])),
        ),
    ]);
    let video = status_response(&encoded(&body), None).expect("the completed operation converts");
    assert_eq!(video.status, "completed");
    assert_eq!(video.id, OPERATION);
}

#[rstest]
#[case::trailing_slash(
    "https://generativelanguage.googleapis.com/",
    "https://generativelanguage.googleapis.com/v1beta/operations/generate_1234567890"
)]
#[case::plain_base(
    "https://generativelanguage.googleapis.com",
    "https://generativelanguage.googleapis.com/v1beta/operations/generate_1234567890"
)]
#[case::versioned_base(
    "https://proxy.test/gemini/v1beta",
    "https://proxy.test/gemini/v1beta/v1beta/operations/generate_1234567890"
)]
fn status_request_unwraps_the_encoded_id(#[case] api_base: &str, #[case] expected: &str) {
    let request = status_request(STATUS_ID, api_base);
    assert_eq!(request.url, expected);
    assert!(request.params.is_empty());
    assert_eq!(decode_video_id_with_provider(STATUS_ID).video_id, OPERATION);
}

#[test]
fn an_unencoded_status_id_stays_as_written() {
    let request = status_request(OPERATION, "https://proxy.test");
    assert_eq!(
        request.url,
        "https://proxy.test/v1beta/operations/generate_1234567890"
    );
}

#[rstest]
#[case::remix_request(
    UnsupportedOperation::RemixRequest,
    "Video remix is not supported by Google Veo. Please use video_generation() to create new videos."
)]
#[case::remix_response(
    UnsupportedOperation::RemixResponse,
    "Video remix is not supported by Google Veo."
)]
#[case::list_request(
    UnsupportedOperation::ListRequest,
    "Video list is not supported by Google Veo. Use the operations endpoint directly if you need to list operations."
)]
#[case::list_response(
    UnsupportedOperation::ListResponse,
    "Video list is not supported by Google Veo."
)]
#[case::delete_request(
    UnsupportedOperation::DeleteRequest,
    "Video delete is not supported by Google Veo. Videos are automatically cleaned up by Google."
)]
#[case::delete_response(
    UnsupportedOperation::DeleteResponse,
    "Video delete is not supported by Google Veo."
)]
#[case::create_character_request(
    UnsupportedOperation::CreateCharacterRequest,
    "video create character is not supported for Gemini"
)]
#[case::create_character_response(
    UnsupportedOperation::CreateCharacterResponse,
    "video create character is not supported for Gemini"
)]
#[case::get_character_request(
    UnsupportedOperation::GetCharacterRequest,
    "video get character is not supported for Gemini"
)]
#[case::get_character_response(
    UnsupportedOperation::GetCharacterResponse,
    "video get character is not supported for Gemini"
)]
#[case::edit_request(
    UnsupportedOperation::EditRequest,
    "video edit is not supported for Gemini"
)]
#[case::edit_response(
    UnsupportedOperation::EditResponse,
    "video edit is not supported for Gemini"
)]
#[case::extension_request(
    UnsupportedOperation::ExtensionRequest,
    "video extension is not supported for Gemini"
)]
#[case::extension_response(
    UnsupportedOperation::ExtensionResponse,
    "video extension is not supported for Gemini"
)]
fn unsupported_operations_keep_litellms_messages(
    #[case] operation: UnsupportedOperation,
    #[case] message: &str,
) {
    let error = StudioVeoOperationError::from(operation);
    assert_eq!(error.to_string(), message);
}

#[test]
fn a_body_over_the_response_cap_is_rejected() {
    let body = vec![b' '; MAX_RESPONSE_BYTES + 1];
    assert!(matches!(
        decode_studio_veo_operation(&body),
        Err(StudioVeoOperationError::ResponseTooLarge { .. })
    ));
}

#[test]
fn a_body_at_the_response_cap_is_read_as_json() {
    let body = vec![b' '; MAX_RESPONSE_BYTES];
    assert!(matches!(
        decode_studio_veo_operation(&body),
        Err(StudioVeoOperationError::Json(_))
    ));
}
