#![cfg(test)]

use std::collections::BTreeMap;
use std::io::{self, Cursor, Read, Seek, SeekFrom};

use betterlite_http::proxy::UpstreamResponse;
use betterlite_prompts::arguments::json::{Json, Object};
use http::header::{HeaderMap, HeaderValue};
use rstest::rstest;

use super::error::StudioImageEditError;
use super::factory::{GEMINI_IMAGE_EDIT_CONFIG, get_gemini_image_edit_config};
use super::options::{SUPPORTED_OPENAI_PARAMS, map_openai_params};
use super::request::{EditImages, MULTIPART_FORM_DATA, transform_image_edit_request};
use super::response::transform_image_edit_response;
use super::url::complete_url;
use crate::gemini_image_policy::error::GeminiImagePolicyError;
use crate::gemini_response::decode::MAX_RESPONSE_BYTES;
use crate::studio_image_generation::error::StudioImageError;
use crate::studio_image_generation::response::ImageGenerationResponse;
use crate::studio_image_generation::settings::{DEFAULT_API_BASE, Settings};
use crate::studio_image_generation::usage::ImageGenerationUsage;
use crate::vertex_gemini_edit::request::GeminiEditFile;
use crate::vertex_gemini_images::error::GeminiImageError;

const GEMINI: &str = "gemini-2.5-flash-image";
// a Gemini model outside the 2.5-flash family, whose imageConfig keeps the imageSize
const SIZED: &str = "gemini-2.0-flash";

fn object(source: &str) -> Object {
    serde_json::from_str(source).expect("object source")
}

fn json(source: &str) -> Json {
    serde_json::from_str(source).expect("json source")
}

fn reply(body: &str) -> UpstreamResponse {
    UpstreamResponse {
        status: 200,
        headers: HeaderMap::new(),
        body: body.as_bytes().to_vec(),
    }
}

fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn studio_settings(pairs: &[(&str, &str)]) -> Settings {
    Settings::from_environment(&environment(pairs))
}

fn body(
    model: &str,
    prompt: Option<&str>,
    image: Option<&mut EditImages<'_>>,
    params: &str,
) -> Json {
    let (payload, files) =
        transform_image_edit_request(model, prompt, image, &object(params), &|_| "image/png")
            .expect("the request builds");
    assert!(files.is_empty());
    Json::Object(payload)
}

fn request_error(prompt: Option<&str>, image: Option<&mut EditImages<'_>>) -> StudioImageEditError {
    transform_image_edit_request(GEMINI, prompt, image, &Object::new(), &|_| "image/png")
        .expect_err("the request is refused")
}

fn edited(source: &str) -> ImageGenerationResponse {
    transform_image_edit_response(&reply(source)).expect("the reply transforms")
}

fn encoded(response: &ImageGenerationResponse) -> Json {
    let text = serde_json::to_string(response).expect("the response encodes");
    serde_json::from_str(&text).expect("the response is JSON")
}

#[rstest]
#[case::gemini(GEMINI)]
#[case::imagen("imagen-3.0-generate-002")]
fn the_factory_answers_the_edit_class_for_any_model(#[case] model: &str) {
    let lookup = environment(&[
        ("GEMINI_API_KEY", "env-key"),
        ("GEMINI_API_BASE", "https://studio.test/v1beta"),
    ]);
    let config = get_gemini_image_edit_config(model, &lookup);
    assert_eq!(GEMINI_IMAGE_EDIT_CONFIG, "GeminiImageEditConfig");
    assert_eq!(config.settings.api_base, "https://studio.test/v1beta");
    assert_eq!(
        config
            .settings
            .gemini_api_key
            .as_ref()
            .map(secrecy::ExposeSecret::expose_secret),
        Some("env-key"),
    );
}

#[rstest]
#[case::trailing_slash("https://studio.test/v1beta/")]
#[case::bare("https://studio.test/v1beta")]
#[case::root("https://studio.test")]
fn an_explicit_base_keeps_one_slash_and_asks_for_generate_content(#[case] base: &str) {
    assert_eq!(
        complete_url(GEMINI, Some(base), &studio_settings(&[])),
        format!(
            "{}/models/{GEMINI}:generateContent",
            base.trim_end_matches('/')
        )
    );
}

#[rstest]
#[case::absent(&[], DEFAULT_API_BASE)]
#[case::empty(&[("GEMINI_API_BASE", "")], DEFAULT_API_BASE)]
#[case::explicit(&[("GEMINI_API_BASE", "https://studio.test/v1beta")], "https://studio.test/v1beta")]
fn the_base_falls_back_to_the_studio_endpoint(
    #[case] pairs: &[(&str, &str)],
    #[case] expected: &str,
) {
    let settings = studio_settings(pairs);
    assert_eq!(settings.api_base, expected);
    assert_eq!(
        complete_url(GEMINI, None, &settings),
        format!("{expected}/models/{GEMINI}:generateContent")
    );
}

#[test]
fn the_edit_dialect_advertises_the_three_names() {
    assert_eq!(SUPPORTED_OPENAI_PARAMS, ["n", "size", "imageConfig"]);
}

#[rstest]
#[case::text(
    r#"{"imageConfig":"{\"aspectRatio\":\"16:9\",\"imageSize\":\"2K\"}"}"#,
    SIZED,
    r#"{"imageConfig":{"aspectRatio":"16:9","imageSize":"2K"}}"#
)]
#[case::object(
    r#"{"imageConfig":{"aspectRatio":"3:2"}}"#,
    SIZED,
    r#"{"imageConfig":{"aspectRatio":"3:2"}}"#
)]
#[case::size_on_flash(
    r#"{"size":"1024x1024"}"#,
    GEMINI,
    r#"{"imageConfig":{"aspectRatio":"1:1"}}"#
)]
#[case::size_on_sized(
    r#"{"size":"1024x1024"}"#,
    SIZED,
    r#"{"imageConfig":{"aspectRatio":"1:1","imageSize":"1K"}}"#
)]
#[case::count(r#"{"n":3}"#, GEMINI, r#"{"sampleCount":3}"#)]
#[case::number_config(r#"{"imageConfig":5}"#, GEMINI, "{}")]
#[case::unknown(r#"{"quality":"hd"}"#, GEMINI, "{}")]
fn the_edit_mapping_parses_image_config_text_and_maps_the_declared_names(
    #[case] params: &str,
    #[case] model: &str,
    #[case] expected: &str,
) {
    assert_eq!(
        Json::Object(map_openai_params(&object(params), model).expect("the params map")),
        json(expected)
    );
}

#[rstest]
#[case::unterminated(r#"{"imageConfig":"{oops"}"#)]
#[case::empty(r#"{"imageConfig":""}"#)]
fn an_image_config_string_that_is_not_json_is_a_400(#[case] params: &str) {
    let error = map_openai_params(&object(params), GEMINI).expect_err("the string is not JSON");
    assert!(matches!(
        error,
        StudioImageEditError::Policy(GeminiImagePolicyError::ImageConfigJson(_))
    ));
    assert_eq!(error.status(), Some(400));
    assert!(error.upstream_answer().is_none());
}

#[test]
fn the_edit_declares_json_transport() {
    const { assert!(!MULTIPART_FORM_DATA) };
}

#[test]
fn the_images_come_before_the_prompt_in_one_object_without_a_role() {
    let mut image = EditImages::Many(vec![
        Some(GeminiEditFile::Bytes(b"one")),
        None,
        Some(GeminiEditFile::Bytes(b"two")),
    ]);
    assert_eq!(
        body(GEMINI, Some("blend these"), Some(&mut image), "{}"),
        json(
            r#"{"contents":[{"parts":[{"inlineData":{"mimeType":"image/png","data":"b25l"}},{"inlineData":{"mimeType":"image/png","data":"dHdv"}},{"text":"blend these"}]}],"generationConfig":{"response_modalities":["IMAGE","TEXT"]}}"#
        )
    );
}

#[rstest]
#[case::absent(None)]
#[case::empty(Some(""))]
fn an_absent_or_empty_prompt_adds_no_text_part(#[case] prompt: Option<&str>) {
    let mut image = EditImages::One(GeminiEditFile::Bytes(b"one"));
    assert_eq!(
        body(GEMINI, prompt, Some(&mut image), "{}"),
        json(
            r#"{"contents":[{"parts":[{"inlineData":{"mimeType":"image/png","data":"b25l"}}]}],"generationConfig":{"response_modalities":["IMAGE","TEXT"]}}"#
        )
    );
}

#[test]
fn no_image_any_way_is_refused() {
    assert!(matches!(
        request_error(None, None),
        StudioImageEditError::MissingImage
    ));
    let mut empty_scalar = EditImages::One(GeminiEditFile::Bytes(b""));
    assert!(matches!(
        request_error(None, Some(&mut empty_scalar)),
        StudioImageEditError::MissingImage
    ));
    let mut empty_list = EditImages::Many(Vec::new());
    assert!(matches!(
        request_error(None, Some(&mut empty_list)),
        StudioImageEditError::MissingImage
    ));
    let mut no_members = EditImages::Many(vec![None, None]);
    assert!(matches!(
        request_error(None, Some(&mut no_members)),
        StudioImageEditError::MissingImage
    ));
}

#[test]
fn an_empty_member_of_an_image_list_still_becomes_a_part() {
    let mut image = EditImages::Many(vec![Some(GeminiEditFile::Bytes(b""))]);
    assert_eq!(
        body(GEMINI, None, Some(&mut image), "{}"),
        json(
            r#"{"contents":[{"parts":[{"inlineData":{"mimeType":"image/png","data":""}}]}],"generationConfig":{"response_modalities":["IMAGE","TEXT"]}}"#
        )
    );
}

#[test]
fn an_unsupported_image_record_is_refused() {
    let mut image = EditImages::One(GeminiEditFile::Unsupported);
    let error = request_error(None, Some(&mut image));
    assert!(matches!(error, StudioImageEditError::UnsupportedImage));
    assert_eq!(error.status(), None);
    assert!(error.upstream_answer().is_none());
}

#[test]
fn a_seekable_image_is_read_from_zero_and_its_cursor_restored() {
    let mut content = Cursor::new(b"0123456789abcdef".to_vec());
    content.seek(SeekFrom::Start(7)).expect("the cursor moves");
    let mut image = EditImages::Many(vec![Some(GeminiEditFile::Seekable(&mut content))]);
    assert_eq!(
        body(GEMINI, None, Some(&mut image), "{}"),
        json(
            r#"{"contents":[{"parts":[{"inlineData":{"mimeType":"image/png","data":"MDEyMzQ1Njc4OWFiY2RlZg=="}}]}],"generationConfig":{"response_modalities":["IMAGE","TEXT"]}}"#
        )
    );
    assert_eq!(content.position(), 7);
}

struct BrokenStream;

impl Read for BrokenStream {
    fn read(&mut self, _buffer: &mut [u8]) -> io::Result<usize> {
        Err(io::Error::other("read refused"))
    }
}

impl Seek for BrokenStream {
    fn seek(&mut self, _position: SeekFrom) -> io::Result<u64> {
        Err(io::Error::other("seek refused"))
    }
}

#[test]
fn a_stream_that_refuses_to_rewind_is_a_stream_error() {
    let mut stream = BrokenStream;
    let mut image = EditImages::One(GeminiEditFile::Seekable(&mut stream));
    let error = request_error(None, Some(&mut image));
    assert!(matches!(error, StudioImageEditError::Stream(_)));
    assert!(error.upstream_answer().is_none());
}

#[test]
fn the_injected_mime_reader_answers_for_each_image() {
    let mut image = EditImages::Many(vec![
        Some(GeminiEditFile::Bytes(b"GIF89a")),
        Some(GeminiEditFile::Bytes(b"\x89PNG")),
    ]);
    let (payload, _files) =
        transform_image_edit_request(GEMINI, None, Some(&mut image), &Object::new(), &|content| {
            if content.starts_with(b"GIF8") {
                "image/gif"
            } else {
                "image/png"
            }
        })
        .expect("the request builds");
    assert_eq!(
        Json::Object(payload),
        json(
            r#"{"contents":[{"parts":[{"inlineData":{"mimeType":"image/gif","data":"R0lGODlh"}},{"inlineData":{"mimeType":"image/png","data":"iVBORw=="}}]}],"generationConfig":{"response_modalities":["IMAGE","TEXT"]}}"#
        )
    );
}

#[test]
fn the_generation_config_carries_the_mapped_count_and_image_config() {
    let mapped =
        object(r#"{"sampleCount":2,"imageConfig":{"aspectRatio":"16:9","imageSize":"4K"}}"#);
    let mut image = EditImages::One(GeminiEditFile::Bytes(b"one"));
    let (payload, _files) =
        transform_image_edit_request(GEMINI, None, Some(&mut image), &mapped, &|_| "image/png")
            .expect("the request builds");
    assert_eq!(
        payload.get("generationConfig"),
        Some(&json(
            r#"{"response_modalities":["IMAGE","TEXT"],"imageConfig":{"aspectRatio":"16:9"},"candidateCount":2}"#
        ))
    );
}

#[test]
fn truthy_inline_data_only_becomes_an_image_and_no_signature_rides_along() {
    let body = r#"{"candidates":[{"content":{"parts":[{"inlineData":{"mimeType":"image/png","data":"AAA"},"thoughtSignature":"sig"},{"text":"ignored"},{"inlineData":{"data":""}}]}},{"content":{"parts":[{"inlineData":{"data":"BBB"}}]}},{"finishReason":"STOP"}]}"#;
    assert_eq!(
        encoded(&edited(body)),
        json(r#"{"data":[{"b64_json":"AAA"},{"b64_json":"BBB"}]}"#)
    );
}

#[rstest]
#[case::empty_string(r#"{"candidates":[{"content":{"parts":[{"inlineData":{"data":""}}]}}]}"#)]
#[case::null_data(r#"{"candidates":[{"content":{"parts":[{"inlineData":{"data":null}}]}}]}"#)]
#[case::empty_inline_data(r#"{"candidates":[{"content":{"parts":[{"inlineData":{}}]}}]}"#)]
#[case::absent_inline_data(r#"{"candidates":[{"content":{"parts":[{"text":"x"}]}}]}"#)]
#[case::missing_content(r#"{"candidates":[{"finishReason":"STOP"}]}"#)]
#[case::no_candidates("{}")]
#[case::null_candidates(r#"{"candidates":null}"#)]
fn a_reply_without_truthy_image_data_holds_no_images(#[case] body: &str) {
    assert_eq!(encoded(&edited(body)), json(r#"{"data":[]}"#));
}

#[test]
fn a_usage_metadata_with_no_images_still_reaches_the_usage() {
    let body = r#"{"candidates":[],"usageMetadata":{"promptTokenCount":4,"candidatesTokenCount":6,"totalTokenCount":10,"promptTokensDetails":[{"modality":"TEXT","tokenCount":4}]}}"#;
    assert_eq!(
        encoded(&edited(body)),
        json(
            r#"{"data":[],"usage":{"input_tokens":4,"input_tokens_details":{"image_tokens":0,"text_tokens":4},"output_tokens":6,"total_tokens":10,"prompt_tokens":4,"prompt_tokens_details":{"image_tokens":0,"text_tokens":4},"completion_tokens":6,"completion_tokens_details":{"image_tokens":6,"text_tokens":0},"output_tokens_details":{"image_tokens":6,"text_tokens":0}}}"#
        )
    );
}

// the edit transform hands usageMetadata to the shared transformer and never counts the searches
#[test]
fn an_edit_usage_counts_no_web_search() {
    let body = r#"{"candidates":[{"content":{"parts":[{"inlineData":{"data":"AAA"}}]},"groundingMetadata":{"webSearchQueries":["pixel art"]}}],"usageMetadata":{"promptTokenCount":4,"candidatesTokenCount":6,"totalTokenCount":10,"promptTokensDetails":[{"modality":"TEXT","tokenCount":4}]}}"#;
    assert_eq!(
        encoded(&edited(body)),
        json(
            r#"{"data":[{"b64_json":"AAA"}],"usage":{"input_tokens":4,"input_tokens_details":{"image_tokens":0,"text_tokens":4},"output_tokens":6,"total_tokens":10,"prompt_tokens":4,"prompt_tokens_details":{"image_tokens":0,"text_tokens":4},"completion_tokens":6,"completion_tokens_details":{"image_tokens":6,"text_tokens":0},"output_tokens_details":{"image_tokens":6,"text_tokens":0}}}"#
        )
    );
}

#[rstest]
#[case::absent(r#"{"candidates":[]}"#)]
#[case::null(r#"{"candidates":[],"usageMetadata":null}"#)]
fn an_absent_or_null_usage_metadata_keeps_usage_away(#[case] body: &str) {
    let response = edited(body);
    assert_eq!(response.usage, None);
    assert_eq!(encoded(&response), json(r#"{"data":[]}"#));
}

#[test]
fn an_empty_usage_metadata_makes_a_zero_usage() {
    let response = edited(r#"{"candidates":[],"usageMetadata":{}}"#);
    assert_eq!(response.usage, Some(ImageGenerationUsage::default()));
}

#[rstest]
#[case::text(r#""x""#)]
#[case::number("5")]
#[case::list("[]")]
fn a_non_object_usage_metadata_is_refused(#[case] metadata: &str) {
    let body = format!(r#"{{"candidates":[],"usageMetadata":{metadata}}}"#);
    let error = transform_image_edit_response(&reply(&body))
        .expect_err("the usage type is declared as an object");
    assert!(matches!(
        error,
        StudioImageEditError::Usage(StudioImageError::UsageShape)
    ));
}

#[test]
fn a_string_token_count_surfaces_the_usage_shape_error() {
    let error =
        transform_image_edit_response(&reply(r#"{"usageMetadata":{"promptTokenCount":"4"}}"#))
            .expect_err("the count type is declared as an integer");
    assert!(matches!(
        &error,
        StudioImageEditError::Usage(StudioImageError::Shape { path, .. }) if path == "promptTokenCount"
    ));
    assert!(error.upstream_answer().is_none());
}

#[test]
fn edit_token_details_that_leave_the_signed_range_are_refused() {
    let prompt = format!(
        r#"{{"candidates":[],"usageMetadata":{{"promptTokensDetails":[{{"modality":"TEXT","tokenCount":{}}},{{"modality":"TEXT","tokenCount":{}}}]}}}}"#,
        i64::MAX,
        i64::MAX
    );
    let error = transform_image_edit_response(&reply(&prompt)).expect_err("the counts overflow");
    assert!(matches!(
        error,
        StudioImageEditError::Usage(StudioImageError::TokenOverflow {
            field: "promptTokensDetails"
        })
    ));
    let candidates = format!(
        r#"{{"candidates":[],"usageMetadata":{{"candidatesTokensDetails":[{{"modality":"IMAGE","tokenCount":{}}},{{"modality":"IMAGE","tokenCount":{}}}]}}}}"#,
        i64::MAX,
        i64::MAX
    );
    let candidate_error =
        transform_image_edit_response(&reply(&candidates)).expect_err("the counts overflow");
    assert!(matches!(
        candidate_error,
        StudioImageEditError::Usage(StudioImageError::TokenOverflow {
            field: "candidatesTokensDetails"
        })
    ));
}

#[test]
fn a_reply_that_is_not_an_object_names_the_shape_and_keeps_no_answer() {
    let error = transform_image_edit_response(&reply("[]")).expect_err("a list root is no reply");
    assert!(matches!(
        error,
        StudioImageEditError::Reply(GeminiImageError::Shape { .. })
    ));
    assert!(error.upstream_answer().is_none());
}

#[test]
fn a_non_json_reply_keeps_the_upstream_answer() {
    let mut headers = HeaderMap::new();
    headers.insert("x-request-id", HeaderValue::from_static("req-1"));
    let truncated = UpstreamResponse {
        status: 502,
        headers,
        body: br#"{"candidates": ["#.to_vec(),
    };
    let error = transform_image_edit_response(&truncated).expect_err("a truncated reply fails");
    let (status, kept) = error
        .upstream_answer()
        .expect("a parse failure keeps the reply answer");
    assert_eq!(status, 502);
    assert_eq!(
        kept.get("x-request-id"),
        Some(&HeaderValue::from_static("req-1"))
    );
}

#[test]
fn a_reply_past_the_shared_cap_is_refused() {
    let mut over = reply("");
    over.body = vec![b'{'; MAX_RESPONSE_BYTES + 1];
    let error = transform_image_edit_response(&over).expect_err("the body is over the cap");
    assert!(matches!(
        error,
        StudioImageEditError::Reply(GeminiImageError::TooLarge { size }) if size == MAX_RESPONSE_BYTES + 1
    ));
}
