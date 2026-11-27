#![cfg(test)]

use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_core::api::bfl::edit::media::SuppliedMedia;
use betterlite_core::api::video_character_calls::error::CharacterProviderFailure as _;
use betterlite_core::api::video_character_calls::{CreateCharacterCall, GetCharacterCall};
use betterlite_core::api::video_character_transport::config::{
    CharacterConfig, CharacterContext, CharacterEnvironment,
};
use betterlite_core::api::video_edit_call::VideoEditConfigLookup;
use betterlite_core::api::video_edit_transport::config::{
    BaseVideoConfig, Headers, Params, VideoEditInput, VideoEnvironment, VideoExtensionInput,
    VideoLogging, VideoReply, VideoRequestContext,
};
use betterlite_core::api::video_extension_call::VideoExtensionConfigLookup;
use betterlite_core::api::video_selection::VideoConfigTag;
use betterlite_http::interactions::base::{
    ErrorClasses, Params as HttpParams, RequestHeaders, ResponseHeaders,
};
use betterlite_http::proxy::UpstreamResponse;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;
use betterlite_openai::openai_video_identifiers::ids::{
    decode_video_id_with_provider, encode_character_id, encode_video_id_with_provider,
};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use http::HeaderMap;
use indexmap::IndexMap;
use secrecy::SecretString;

use crate::video_edits::adapter::OpenAiEditConfig;
use crate::video_edits::session::logging as call_logging;

use super::requests::{Build, OwnedEdit, OwnedExtension};
use super::{ConvertError, VideoEditWire, VideoFile, parse};

fn wire(arguments: &str) -> VideoEditWire {
    let payload = format!("{{\"arguments\":{arguments}}}");
    parse(&payload, &mut []).unwrap().0
}

fn build<Request>(arguments: &str) -> Result<Request, ConvertError>
where
    Request: Build,
{
    Request::build(&wire(arguments).arguments, None)
}

fn parsed(arguments: &str, blobs: &mut [Vec<u8>]) -> (VideoEditWire, Option<VideoFile>) {
    let payload = format!("{{\"arguments\":{arguments}}}");
    parse(&payload, blobs).unwrap()
}

fn create(arguments: &str) -> CreateCharacterCall {
    build(arguments).unwrap()
}

fn get(arguments: &str) -> GetCharacterCall {
    build(arguments).unwrap()
}

fn edit(arguments: &str) -> OwnedEdit {
    build(arguments).unwrap()
}

fn extension(arguments: &str) -> OwnedExtension {
    build(arguments).unwrap()
}

fn openai_config(video_name: Option<&str>) -> OpenAiEditConfig {
    OpenAiEditConfig {
        environment: BTreeMap::new(),
        global_api_key: None,
        openai_key: None,
        video_name: video_name.map(str::to_owned),
    }
}

const VIDEOS_BASE: &str = "http://host.test/v1/videos";

fn character_context<'call>(
    params: &'call HttpParams,
    headers: &'call RequestHeaders,
) -> CharacterContext<'call> {
    CharacterContext {
        api_base: VIDEOS_BASE,
        litellm_params: params,
        headers,
    }
}

fn request_context<'call>(
    params: &'call Params,
    headers: &'call Headers,
) -> VideoRequestContext<'call> {
    VideoRequestContext {
        headers,
        litellm_params: params,
        api_base: VIDEOS_BASE,
    }
}

fn logging_state() -> LoggingState {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    call_logging(&redactor, "video_create_character").unwrap()
}

#[test]
fn request_create_character_arguments_reach_the_call() {
    let request = create(
        r#"{"name": "hero", "custom_llm_provider": "openai", "timeout": 12.5,
            "extra_headers": {"x-one": "1"}, "extra_query": {"q": "1"},
            "extra_body": {"watermark": "yes"}, "litellm_call_id": "call-1"}"#,
    );
    assert_eq!(request.name, "hero");
    assert_eq!(request.timeout, Some(12.5_f64));
    assert_eq!(request.custom_llm_provider, "openai");
    assert_eq!(
        request
            .extra_headers
            .unwrap()
            .get("x-one")
            .map(String::as_str),
        Some("1")
    );
    assert_eq!(request.litellm_call_id.as_deref(), Some("call-1"));
    assert_eq!(request.extra_query, None);
    assert_eq!(request.extra_body, None);
    assert!(request.extra.is_empty());
}

#[test]
fn request_create_character_leaves_the_timeout_to_the_core() {
    assert_eq!(create(r#"{"name": "hero"}"#).timeout, None);
    assert_eq!(create(r#"{"name": "hero", "timeout": null}"#).timeout, None);
}

#[test]
fn request_create_character_defaults_the_provider_to_openai() {
    assert_eq!(create(r#"{"name": "hero"}"#).custom_llm_provider, "openai");
}

#[test]
fn request_create_character_text_video_is_carried_as_sent() {
    let request = create(r#"{"name": "hero", "video": "https://cdn.test/a.mp4"}"#);
    assert_eq!(
        request.video,
        SuppliedMedia::Text(String::from("https://cdn.test/a.mp4"))
    );
}

#[test]
fn request_create_character_defaults_a_missing_video() {
    assert_eq!(
        create(r#"{"name": "hero"}"#).video,
        SuppliedMedia::Text(String::new())
    );
}

#[test]
fn request_create_character_video_bytes_carry_the_payload_name() {
    let mut blobs = vec![b"\x00\x00\x00\x18ftypmp42".to_vec()];
    let (wire, file) = parsed(
        r#"{"name": "hero", "video": {"$file": {"name": "clip.mov", "blob": 0, "content_type": null}}}"#,
        &mut blobs,
    );
    let media_file = file.unwrap();
    assert_eq!(media_file.name.as_deref(), Some("clip.mov"));
    let request = CreateCharacterCall::build(&wire.arguments, Some(&media_file)).unwrap();
    assert_eq!(
        request.video,
        SuppliedMedia::Bytes(b"\x00\x00\x00\x18ftypmp42".to_vec())
    );
}

#[test]
fn request_create_character_default_file_name_is_unnamed() {
    let mut blobs = vec![b"video bytes".to_vec()];
    let (_wire, file) = parsed(
        r#"{"name": "hero", "video": {"$file": {"name": "file", "blob": 0, "content_type": null}}}"#,
        &mut blobs,
    );
    assert_eq!(file.unwrap().name, None);
}

#[test]
fn request_create_character_loose_keywords_stay_in_extra() {
    let request = create(
        r#"{"name": "hero", "user": "u-1", "api_base": "http://host.test/v1",
            "mock_response": {"id": "c", "object": "character", "created_at": 1, "name": "n"}}"#,
    );
    assert_eq!(
        request.extra.get("api_base"),
        Some(&Json::Text(String::from("http://host.test/v1")))
    );
    assert_eq!(
        request.extra.get("user"),
        Some(&Json::Text(String::from("u-1")))
    );
    assert!(request.mock_response.is_some());
}

#[test]
fn request_get_character_arguments_reach_the_call() {
    let request = get(
        r#"{"character_id": "char-1", "custom_llm_provider": "openai", "timeout": 5,
            "extra_headers": {"x-one": "1"}, "extra_query": {"q": "1"}}"#,
    );
    assert_eq!(request.character_id, "char-1");
    assert_eq!(request.timeout, Some(5.0_f64));
    assert_eq!(request.custom_llm_provider, "openai");
    assert_eq!(
        request
            .extra_headers
            .unwrap()
            .get("x-one")
            .map(String::as_str),
        Some("1")
    );
    assert_eq!(request.extra_query, None);
    assert!(request.extra.is_empty());
}

#[test]
fn request_get_character_ignores_a_video_file() {
    let mut blobs = vec![b"video bytes".to_vec()];
    let (wire, file) = parsed(
        r#"{"character_id": "char-1", "video": {"$file": {"name": "clip.mp4", "blob": 0, "content_type": null}}}"#,
        &mut blobs,
    );
    let request = GetCharacterCall::build(&wire.arguments, file.as_ref()).unwrap();
    assert_eq!(request.character_id, "char-1");
}

#[test]
fn request_edit_arguments_reach_the_call() {
    let request = edit(
        r#"{"video_id": "vid-1", "prompt": "add rain", "timeout": 1.5,
            "custom_llm_provider": "azure", "extra_headers": {"x-one": "1"},
            "extra_body": {"watermark": "yes"}, "user": "u-1"}"#,
    );
    let call = request.call();
    assert_eq!(call.video_id, "vid-1");
    assert_eq!(call.prompt, "add rain");
    assert_eq!(call.timeout, Some(Duration::from_millis(1500)));
    assert_eq!(call.custom_llm_provider, Some("azure"));
    assert_eq!(
        call.extra_headers.unwrap().get("x-one").map(String::as_str),
        Some("1")
    );
    assert_eq!(
        call.extra_body.unwrap().get("watermark"),
        Some(&Json::Text(String::from("yes")))
    );
    assert_eq!(
        call.kwargs.get("user"),
        Some(&Json::Text(String::from("u-1")))
    );
    assert_eq!(call.extra_query, None);
}

#[test]
fn request_edit_keeps_the_logging_object_in_kwargs() {
    let request = edit(r#"{"video_id": "vid-1", "prompt": "p", "async_call": true}"#);
    let call = request.call();
    assert_eq!(
        call.kwargs.get("litellm_logging_obj"),
        Some(&Json::Bool(true))
    );
    assert!(!call.kwargs.contains_key("video_id"));
    assert!(!call.kwargs.contains_key("prompt"));
    assert!(!call.kwargs.contains_key("timeout"));
    assert!(!call.kwargs.contains_key("extra_body"));
    assert!(!call.kwargs.contains_key("async_call"));
}

#[test]
fn request_edit_text_video_is_carried_as_content() {
    let request = edit(r#"{"video_id": "vid-1", "prompt": "p", "video": "a.mp4"}"#);
    assert_eq!(
        request.call().video,
        Some(&SuppliedMedia::Text(String::from("a.mp4")))
    );
}

#[test]
fn request_edit_video_file_is_carried_in_the_call() {
    let mut blobs = vec![b"video bytes".to_vec()];
    let (wire, file) = parsed(
        r#"{"video_id": "vid-1", "prompt": "p", "video": {"$file": {"name": "clip.mp4", "blob": 0, "content_type": null}}}"#,
        &mut blobs,
    );
    let media_file = file.unwrap();
    assert_eq!(media_file.name.as_deref(), Some("clip.mp4"));
    let request = OwnedEdit::build(&wire.arguments, Some(&media_file)).unwrap();
    assert_eq!(
        request.call().video,
        Some(&SuppliedMedia::Bytes(b"video bytes".to_vec()))
    );
}

#[test]
fn request_extension_arguments_reach_the_call() {
    let request = extension(
        r#"{"video_id": "vid-1", "prompt": "more", "seconds": "8", "timeout": 2,
            "custom_llm_provider": "openai", "extra_headers": {"x-one": "1"},
            "extra_body": {"watermark": "yes"}}"#,
    );
    let call = request.call();
    assert_eq!(call.video_id, "vid-1");
    assert_eq!(call.prompt, "more");
    assert_eq!(call.seconds, "8");
    assert_eq!(call.timeout, Some(Duration::from_secs(2)));
    assert_eq!(call.custom_llm_provider, Some("openai"));
    assert_eq!(
        call.extra_body.unwrap().get("watermark"),
        Some(&Json::Text(String::from("yes")))
    );
    assert_eq!(
        call.kwargs.get("litellm_logging_obj"),
        Some(&Json::Bool(true))
    );
}

#[test]
fn request_extension_seconds_are_not_parsed() {
    assert_eq!(
        extension(r#"{"video_id": "vid-1", "prompt": "p", "seconds": "eight"}"#)
            .call()
            .seconds,
        "eight"
    );
}

#[test]
fn error_create_character_without_a_name_names_the_field() {
    let error = build::<CreateCharacterCall>("{}").err().unwrap();
    assert_eq!(error.to_string(), "name must be a string");
}

#[test]
fn error_get_character_without_an_id_names_the_field() {
    let error = build::<GetCharacterCall>("{}").err().unwrap();
    assert_eq!(error.to_string(), "character_id must be a string");
}

#[test]
fn error_edit_without_a_prompt_names_the_field() {
    let error = build::<OwnedEdit>(r#"{"video_id": "vid-1"}"#)
        .err()
        .unwrap();
    assert_eq!(error.to_string(), "prompt must be a string");
}

#[test]
fn error_edit_without_a_video_id_names_the_field() {
    let error = build::<OwnedEdit>(r#"{"prompt": "p"}"#).err().unwrap();
    assert_eq!(error.to_string(), "video_id must be a string");
}

#[test]
fn error_extension_without_seconds_names_the_field() {
    let error = build::<OwnedExtension>(r#"{"video_id": "vid-1", "prompt": "p"}"#)
        .err()
        .unwrap();
    assert_eq!(error.to_string(), "seconds must be a string");
}

#[test]
fn error_timeout_that_is_not_a_number_names_the_field() {
    let error = build::<OwnedEdit>(r#"{"video_id": "v", "prompt": "p", "timeout": "soon"}"#)
        .err()
        .unwrap();
    assert_eq!(error.to_string(), "timeout must be a number of seconds");
}

#[test]
fn error_timeout_out_of_range_is_reported() {
    let error = build::<OwnedEdit>(r#"{"video_id": "v", "prompt": "p", "timeout": 1e300}"#)
        .err()
        .unwrap();
    assert!(matches!(error, ConvertError::Timeout { .. }));
}

#[test]
fn error_header_value_that_is_not_a_string_names_the_field() {
    let error = build::<CreateCharacterCall>(r#"{"name": "hero", "extra_headers": {"x": 1}}"#)
        .err()
        .unwrap();
    assert_eq!(error.to_string(), "extra_headers must be a map of strings");
}

#[test]
fn error_video_that_is_not_a_file_names_the_field() {
    let error = build::<CreateCharacterCall>(r#"{"name": "hero", "video": 4}"#)
        .err()
        .unwrap();
    assert_eq!(error.to_string(), "video must be a file");
}

#[test]
fn error_extra_body_that_is_not_a_map_names_the_field() {
    let error = build::<OwnedEdit>(r#"{"video_id": "v", "prompt": "p", "extra_body": 4}"#)
        .err()
        .unwrap();
    assert_eq!(error.to_string(), "extra_body must be a map");
}

#[test]
fn error_payload_that_is_not_json_is_reported() {
    let error = parse("not json", &mut []).err().unwrap();
    assert!(
        error
            .to_string()
            .starts_with("the native request text is not JSON")
    );
}

#[test]
fn request_adapter_builds_the_character_create_request() {
    let config = openai_config(None);
    let media = SuppliedMedia::Bytes(b"\x00\x00\x00\x18ftypmp42".to_vec());
    let params = HttpParams::new();
    let headers = RequestHeaders::new();
    let (url, parts) = config
        .transform_create_request("hero", &media, character_context(&params, &headers))
        .unwrap();
    assert_eq!(url, "http://host.test/v1/videos/characters");
    assert_eq!(parts.len(), 2);
    assert_eq!(parts[0].name, "name");
    assert_eq!(parts[0].file_name, None);
    assert_eq!(parts[0].data, b"hero".to_vec());
    assert_eq!(parts[1].name, "video");
    assert_eq!(parts[1].file_name.as_deref(), Some("input_video.mp4"));
    assert_eq!(parts[1].content_type.as_deref(), Some("video/mp4"));
    assert_eq!(parts[1].data, b"\x00\x00\x00\x18ftypmp42".to_vec());
}

#[test]
fn request_adapter_uses_the_callers_file_name() {
    let config = openai_config(Some("clip.mov"));
    let media = SuppliedMedia::Bytes(b"ftyp".to_vec());
    let params = HttpParams::new();
    let headers = RequestHeaders::new();
    let (_url, parts) = config
        .transform_create_request("hero", &media, character_context(&params, &headers))
        .unwrap();
    assert_eq!(parts[1].file_name.as_deref(), Some("clip.mov"));
    assert_eq!(parts[1].content_type.as_deref(), Some("video/quicktime"));
}

#[test]
fn request_adapter_builds_the_character_lookup_url() {
    let config = openai_config(None);
    let params = HttpParams::new();
    let headers = RequestHeaders::new();
    assert_eq!(
        config
            .transform_get_request("char-1", character_context(&params, &headers))
            .unwrap()
            .0,
        "http://host.test/v1/videos/characters/char-1"
    );
    let encoded = encode_character_id("char-2", "openai", None);
    assert_eq!(
        config
            .transform_get_request(&encoded, character_context(&params, &headers))
            .unwrap()
            .0,
        "http://host.test/v1/videos/characters/char-2"
    );
}

#[test]
fn request_adapter_builds_the_edit_form_for_a_file() {
    let config = openai_config(Some("clip.mp4"));
    let media = SuppliedMedia::Bytes(b"video bytes".to_vec());
    let extra = IndexMap::from([(String::from("watermark"), Json::Text(String::from("yes")))]);
    let params = Params::new();
    let headers = Headers::new();
    let (url, body, files) = config
        .transform_video_edit_request(
            VideoEditInput {
                prompt: "add rain",
                video_id: "vid-1",
                video_file: Some(&media),
                extra_body: Some(&extra),
                prefetched_source_data: None,
            },
            request_context(&params, &headers),
        )
        .unwrap();
    assert_eq!(url, "http://host.test/v1/videos/edits");
    assert_eq!(
        body.get("prompt"),
        Some(&Json::Text(String::from("add rain")))
    );
    assert_eq!(
        body.get("watermark"),
        Some(&Json::Text(String::from("yes")))
    );
    let request_files = files.unwrap();
    assert_eq!(request_files.len(), 1);
    assert_eq!(request_files[0].field, "video");
    assert_eq!(request_files[0].filename.as_deref(), Some("clip.mp4"));
    assert_eq!(request_files[0].content_type.as_deref(), Some("video/mp4"));
    assert_eq!(request_files[0].content, b"video bytes".to_vec());
}

#[test]
fn request_adapter_builds_the_edit_body_for_a_video_id() {
    let config = openai_config(None);
    let params = Params::new();
    let headers = Headers::new();
    let encoded = encode_video_id_with_provider("vid-1", "openai", None);
    let (url, body, files) = config
        .transform_video_edit_request(
            VideoEditInput {
                prompt: "add rain",
                video_id: &encoded,
                video_file: None,
                extra_body: None,
                prefetched_source_data: None,
            },
            request_context(&params, &headers),
        )
        .unwrap();
    assert_eq!(url, "http://host.test/v1/videos/edits");
    assert!(files.is_none());
    assert_eq!(
        body.get("video"),
        Some(&Json::Object(IndexMap::from([(
            String::from("id"),
            Json::Text(String::from("vid-1")),
        )])))
    );
}

#[test]
fn request_adapter_builds_the_extension_body() {
    let config = openai_config(None);
    let extra = IndexMap::from([(String::from("watermark"), Json::Text(String::from("yes")))]);
    let params = Params::new();
    let headers = Headers::new();
    let (url, body) = config
        .transform_video_extension_request(
            VideoExtensionInput {
                prompt: "more",
                video_id: "vid-1",
                seconds: "8",
                extra_body: Some(&extra),
            },
            request_context(&params, &headers),
        )
        .unwrap();
    assert_eq!(url, "http://host.test/v1/videos/extensions");
    assert_eq!(body.get("prompt"), Some(&Json::Text(String::from("more"))));
    assert_eq!(body.get("seconds"), Some(&Json::Text(String::from("8"))));
    assert_eq!(
        body.get("watermark"),
        Some(&Json::Text(String::from("yes")))
    );
}

#[test]
fn request_adapter_wraps_the_edit_reply_with_the_provider() {
    let config = openai_config(None);
    let reply = UpstreamResponse {
        status: 200,
        headers: HeaderMap::new(),
        body: br#"{"id": "vid-1", "object": "video", "status": "queued"}"#.to_vec(),
    };
    let mut logging = VideoLogging::default();
    let object = config
        .transform_video_edit_response(
            VideoReply {
                raw_response: &reply,
                logging_obj: &mut logging,
                custom_llm_provider: Some("openai"),
            },
            None,
        )
        .unwrap();
    assert_eq!(object.status, "queued");
    assert_eq!(decode_video_id_with_provider(&object.id).video_id, "vid-1");
    assert_eq!(
        decode_video_id_with_provider(&object.id)
            .custom_llm_provider
            .as_deref(),
        Some("openai")
    );
}

#[test]
fn request_adapter_decodes_the_character_replies() {
    let config = openai_config(None);
    let reply = UpstreamResponse {
        status: 200,
        headers: HeaderMap::new(),
        body: br#"{"id": "char-1", "object": "character", "created_at": 7, "name": "hero"}"#
            .to_vec(),
    };
    let mut state = logging_state();
    let created = config
        .transform_create_response(&reply, &mut state)
        .unwrap();
    assert_eq!(created.id, "char-1");
    assert_eq!(created.created_at, 7);
    assert_eq!(created.name, "hero");
    let looked_up = config.transform_get_response(&reply, &mut state).unwrap();
    assert_eq!(looked_up.id, "char-1");
    assert_eq!(looked_up.name, "hero");
}

#[test]
fn request_adapter_wraps_the_extension_reply_with_the_provider() {
    let config = openai_config(None);
    let reply = UpstreamResponse {
        status: 200,
        headers: HeaderMap::new(),
        body: br#"{"id": "vid-1", "object": "video", "status": "queued"}"#.to_vec(),
    };
    let mut logging = VideoLogging::default();
    let object = config
        .transform_video_extension_response(VideoReply {
            raw_response: &reply,
            logging_obj: &mut logging,
            custom_llm_provider: Some("openai"),
        })
        .unwrap();
    assert_eq!(object.status, "queued");
    assert_eq!(decode_video_id_with_provider(&object.id).video_id, "vid-1");
    assert_eq!(
        decode_video_id_with_provider(&object.id)
            .custom_llm_provider
            .as_deref(),
        Some("openai")
    );
}

#[test]
fn request_adapter_maps_provider_errors() {
    let config = openai_config(None);
    let failure = BaseVideoConfig::get_error_class(&config, "boom", 429, &Headers::new());
    assert_eq!(failure.status_code, 429);
    assert_eq!(failure.message, "boom");
}

#[test]
fn request_adapter_serves_only_the_openai_tag() {
    let config = openai_config(None);
    VideoEditConfigLookup::video_config(&config, &VideoConfigTag::OpenAi).unwrap();
    assert!(VideoEditConfigLookup::video_config(&config, &VideoConfigTag::Gemini).is_none());
    VideoExtensionConfigLookup::video_config(&config, &VideoConfigTag::OpenAi).unwrap();
    assert!(VideoExtensionConfigLookup::video_config(&config, &VideoConfigTag::FalAi).is_none());
}

#[test]
fn request_adapter_reports_the_character_provider_failure() {
    let config = openai_config(None);
    let failure = ErrorClasses::get_error_class(&config, "boom", 429, &ResponseHeaders::new());
    assert_eq!(failure.status_code(), Some(429));
    assert_eq!(failure.message(), "boom");
}

#[test]
fn request_adapter_reports_a_malformed_character_reply() {
    let config = openai_config(None);
    let reply = UpstreamResponse {
        status: 200,
        headers: HeaderMap::new(),
        body: b"not json".to_vec(),
    };
    let mut state = logging_state();
    let error = config
        .transform_create_response(&reply, &mut state)
        .err()
        .unwrap();
    assert!(!error.message.is_empty());
}

#[test]
fn request_adapter_character_headers_use_the_environment_key() {
    let config = OpenAiEditConfig {
        environment: BTreeMap::from([(String::from("OPENAI_API_KEY"), String::from("sk-env"))]),
        global_api_key: None,
        openai_key: None,
        video_name: None,
    };
    let params = HttpParams::new();
    let headers = CharacterConfig::validate_environment(
        &config,
        CharacterEnvironment {
            api_key: None,
            headers: RequestHeaders::new(),
            model: "",
            litellm_params: &params,
        },
    );
    assert_eq!(
        headers
            .get("Authorization")
            .map(secrecy::ExposeSecret::expose_secret),
        Some("Bearer sk-env")
    );
}

#[test]
fn request_adapter_character_headers_use_the_call_key() {
    let config = openai_config(None);
    let mut params = HttpParams::new();
    params.insert(
        String::from("api_key"),
        JsonValue::Text(String::from("sk-params")),
    );
    let headers = CharacterConfig::validate_environment(
        &config,
        CharacterEnvironment {
            api_key: None,
            headers: RequestHeaders::new(),
            model: "",
            litellm_params: &params,
        },
    );
    assert_eq!(
        headers
            .get("Authorization")
            .map(secrecy::ExposeSecret::expose_secret),
        Some("Bearer sk-params")
    );
}

#[test]
fn request_adapter_edit_headers_use_the_call_key() {
    let config = openai_config(None);
    let params = Params::from([(
        String::from("api_key"),
        Json::Text(String::from("sk-params")),
    )]);
    let headers = Headers::new();
    let validated = BaseVideoConfig::validate_environment(
        &config,
        VideoEnvironment {
            headers: &headers,
            model: "",
            api_key: None,
            litellm_params: Some(&params),
        },
    );
    assert_eq!(
        validated.get("Authorization").map(String::as_str),
        Some("Bearer sk-params")
    );
}

#[test]
fn request_adapter_edit_headers_fall_back_to_the_configured_key() {
    let config = OpenAiEditConfig {
        environment: BTreeMap::new(),
        global_api_key: None,
        openai_key: Some(SecretString::from("sk-openai")),
        video_name: None,
    };
    let params = Params::new();
    let headers = Headers::new();
    let validated = BaseVideoConfig::validate_environment(
        &config,
        VideoEnvironment {
            headers: &headers,
            model: "",
            api_key: None,
            litellm_params: Some(&params),
        },
    );
    assert_eq!(
        validated.get("Authorization").map(String::as_str),
        Some("Bearer sk-openai")
    );
}

#[test]
fn request_adapter_completes_the_video_base() {
    let config = openai_config(None);
    let params = HttpParams::new();
    let character_url =
        CharacterConfig::complete_url(&config, "", Some("http://host.test/v1"), &params).unwrap();
    assert_eq!(character_url, "http://host.test/v1/videos");
    let edit_params = Params::new();
    let edit_url = config
        .get_complete_url("", Some("http://host.test/v1"), &edit_params)
        .unwrap();
    assert_eq!(edit_url, "http://host.test/v1/videos");
}

#[test]
fn request_adapter_has_no_edit_prefetch_params() {
    let config = openai_config(None);
    let params = Params::new();
    let headers = Headers::new();
    assert!(
        config
            .get_video_edit_prefetch_params("vid-1", request_context(&params, &headers))
            .is_none()
    );
}
