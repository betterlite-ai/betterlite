#![cfg(test)]

use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_core::api::bfl::edit::media::SuppliedMedia;
use betterlite_core::api::video_content_call::VideoContentRequest;
use betterlite_core::api::video_generation_call::VideoGenerationRequest;
use betterlite_core::api::video_list_call::VideoListCall;
use betterlite_core::api::video_records::VideoCreateOptionalRequestParams;
use betterlite_core::api::video_remix_call::VideoRemixCall;
use betterlite_core::api::video_status_call::VideoStatusRequest;
use betterlite_core::api::videos_http_transport::video_config::{
    Params, VideoEnvironment, VideoHttpConfig, VideoLogging, VideoRequestContext,
};
use betterlite_models::decisions::Json;
use betterlite_sdk::call_files::CallFile;
use indexmap::IndexMap;
use secrecy::ExposeSecret as _;

use crate::video_calls::adapter::OpenAiVideoConfig;
use crate::video_calls::session::{VideoSession, config as session_config};
use betterlite_core::api::video_generation_call::providers::VideoGenerationConfig;
use betterlite_core::api::video_remix_list_transport::config::VideoRemixListConfig;
use betterlite_core::api::video_remix_list_transport::records::{
    VideoListQuery, VideoRequestContext as RemixContext,
};
use betterlite_core::api::video_status_transport::config::VideoStatusConfig;
use betterlite_core::api::video_status_transport::records::VideoRequestContext as StatusContext;
use betterlite_http::proxy::UpstreamResponse;

use super::{
    ConvertError, Operation, Reference, Requests, SettingsWire, VideoCallWire, drop_params, parse,
    request,
};

fn wire(arguments: &str) -> VideoCallWire {
    let payload = format!("{{\"arguments\":{arguments}}}");
    parse(&payload, &mut []).unwrap().0
}

fn build(operation: Operation, arguments: &str) -> Result<Requests, ConvertError> {
    request(operation, &wire(arguments), None)
}

fn generation(arguments: &str) -> VideoGenerationRequest {
    build(Operation::Generate, arguments)
        .unwrap()
        .generation
        .unwrap()
}

fn content(arguments: &str) -> VideoContentRequest {
    build(Operation::Content, arguments)
        .unwrap()
        .content
        .unwrap()
}

fn remix(arguments: &str) -> VideoRemixCall {
    build(Operation::Remix, arguments).unwrap().remix.unwrap()
}

fn list(arguments: &str) -> VideoListCall {
    build(Operation::List, arguments).unwrap().list.unwrap()
}

fn status(arguments: &str) -> VideoStatusRequest {
    build(Operation::Status, arguments).unwrap().status.unwrap()
}

#[test]
fn request_generation_arguments_reach_the_call() {
    let request = generation(
        r#"{"prompt": "a cat", "model": "sora-2", "seconds": "8", "size": "720x1280",
            "user": "u-1", "timeout": 12.5, "custom_llm_provider": "openai",
            "extra_headers": {"x-one": "1"}, "extra_query": {"api-version": "2"},
            "extra_body": {"watermark": "yes"}}"#,
    );
    assert_eq!(request.prompt, "a cat");
    assert_eq!(request.model.as_deref(), Some("sora-2"));
    assert_eq!(request.seconds.as_deref(), Some("8"));
    assert_eq!(request.size.as_deref(), Some("720x1280"));
    assert_eq!(request.user.as_deref(), Some("u-1"));
    assert_eq!(request.timeout, Some(12.5_f64));
    assert_eq!(request.custom_llm_provider.as_deref(), Some("openai"));
    assert_eq!(
        request
            .extra_headers
            .unwrap()
            .get("x-one")
            .map(String::as_str),
        Some("1")
    );
    assert_eq!(
        request.extra_query.unwrap().get("api-version"),
        Some(&Json::Text(String::from("2")))
    );
    assert_eq!(
        request.extra_body.unwrap().get("watermark"),
        Some(&Json::Text(String::from("yes")))
    );
    assert!(request.kwargs.is_empty());
}

#[test]
fn request_generation_seconds_integer_is_sent_as_given() {
    assert_eq!(
        generation(r#"{"prompt": "p", "seconds": 8}"#)
            .seconds
            .as_deref(),
        Some("8")
    );
}

#[test]
fn request_generation_timeout_null_is_none() {
    assert_eq!(
        generation(r#"{"prompt": "p", "timeout": null}"#).timeout,
        None
    );
}

#[test]
fn request_generation_loose_keywords_stay_in_kwargs() {
    let request = generation(
        r#"{"prompt": "p", "api_base": "http://example.test/v1", "api_key": "k",
            "mock_response": "m", "litellm_call_id": "c"}"#,
    );
    assert_eq!(
        request.kwargs.get("api_base"),
        Some(&Json::Text(String::from("http://example.test/v1")))
    );
    assert_eq!(
        request.kwargs.get("api_key"),
        Some(&Json::Text(String::from("k")))
    );
    assert_eq!(
        request.kwargs.get("mock_response"),
        Some(&Json::Text(String::from("m")))
    );
    assert_eq!(
        request.kwargs.get("litellm_call_id"),
        Some(&Json::Text(String::from("c")))
    );
}

#[test]
fn request_generation_input_reference_bytes_carry_their_name() {
    let mut blobs = vec![b"PNG".to_vec()];
    let payload = r#"{"arguments": {"prompt": "p", "input_reference": {"$file": {"blob": 0, "name": "ref.png", "content_type": "image/png"}}}}"#;
    let (wire, supplied) = parse(payload, &mut blobs).unwrap();
    let media = supplied.unwrap();
    assert_eq!(media.name.as_deref(), Some("ref.png"));
    assert_eq!(media.content, b"PNG".to_vec());
    let request = request(Operation::Generate, &wire, Some(media))
        .unwrap()
        .generation
        .unwrap();
    assert_eq!(
        request.input_reference,
        Some(SuppliedMedia::Bytes(b"PNG".to_vec()))
    );
}

#[test]
fn request_generation_default_file_name_is_unnamed() {
    let mut blobs = vec![b"PNG".to_vec()];
    let payload = r#"{"arguments": {"prompt": "p", "input_reference": {"$file": {"blob": 0}}}}"#;
    let (wire, supplied) = parse(payload, &mut blobs).unwrap();
    let media = supplied.unwrap();
    assert_eq!(media.name, None);
    assert_eq!(media.content, b"PNG".to_vec());
    let request = request(Operation::Generate, &wire, Some(media))
        .unwrap()
        .generation
        .unwrap();
    assert_eq!(
        request.input_reference,
        Some(SuppliedMedia::Bytes(b"PNG".to_vec()))
    );
}

#[test]
fn request_content_arguments_reach_the_call() {
    let request = content(
        r#"{"video_id": "video_123", "timeout": 2.5, "custom_llm_provider": "openai",
            "variant": "thumbnail", "extra_headers": {"x-one": "1"}}"#,
    );
    assert_eq!(request.video_id, "video_123");
    assert_eq!(
        request.timeout,
        Some(Duration::try_from_secs_f64(2.5).unwrap())
    );
    assert_eq!(request.custom_llm_provider.as_deref(), Some("openai"));
    assert_eq!(request.variant.as_deref(), Some("thumbnail"));
    assert_eq!(
        request
            .extra_headers
            .unwrap()
            .get("x-one")
            .map(String::as_str),
        Some("1")
    );
}

#[test]
fn request_content_timeout_null_stays_none() {
    assert_eq!(
        content(r#"{"video_id": "v", "timeout": null}"#).timeout,
        None
    );
}

#[test]
fn request_remix_arguments_reach_the_call() {
    let request = remix(
        r#"{"video_id": "video_123", "prompt": "again", "custom_llm_provider": "openai",
            "api_key": "sk-1", "api_base": "http://example.test/v1", "api_version": "2024-01",
            "extra_headers": {"x-one": "1"}, "extra_body": {"seconds": "4"},
            "mock_response": {"id": "v"}, "litellm_metadata": {"user_api_key": "k"},
            "litellm_call_id": "call-1", "user": "u-1", "timeout": 30, "litellm_trace_id": "t-1"}"#,
    );
    assert_eq!(request.video_id, "video_123");
    assert_eq!(request.prompt, "again");
    assert_eq!(request.custom_llm_provider.as_deref(), Some("openai"));
    assert_eq!(request.api_base.as_deref(), Some("http://example.test/v1"));
    assert_eq!(request.api_version.as_deref(), Some("2024-01"));
    assert_eq!(request.user.as_deref(), Some("u-1"));
    assert_eq!(request.timeout, Some(30.0_f64));
    assert_eq!(
        request
            .extra_headers
            .as_ref()
            .unwrap()
            .get("x-one")
            .map(String::as_str),
        Some("1")
    );
    assert_eq!(
        request.extra_body.as_ref().unwrap().get("seconds"),
        Some(&Json::Text(String::from("4")))
    );
    assert_eq!(
        request.mock_response,
        Some(Json::Object(IndexMap::from_iter([(
            String::from("id"),
            Json::Text(String::from("v")),
        )])))
    );
}

#[test]
fn request_remix_metadata_reaches_the_call() {
    let request = remix(
        r#"{"video_id":"v", "prompt":"p", "api_key":"sk-1", "litellm_metadata":{"user_api_key":"k"}, "litellm_call_id":"call-1", "litellm_trace_id":"t-1"}"#,
    );
    assert_eq!(request.litellm_metadata.as_ref().unwrap().len(), 1);
    assert_eq!(request.litellm_call_id.as_deref(), Some("call-1"));
    assert_eq!(
        request.extra.get("litellm_trace_id"),
        Some(&Json::Text(String::from("t-1")))
    );
    assert_eq!(request.api_key.as_ref().unwrap().expose_secret(), "sk-1");
}

#[test]
fn request_list_arguments_reach_the_call() {
    let request = list(
        r#"{"after": "video_1", "limit": 3, "order": "asc", "api_key": "sk-1",
            "timeout": 5, "custom_llm_provider": "openai"}"#,
    );
    assert_eq!(request.after.as_deref(), Some("video_1"));
    assert_eq!(request.limit, Some(3));
    assert_eq!(request.order.as_deref(), Some("asc"));
    assert_eq!(request.timeout, Some(5.0_f64));
    assert_eq!(request.custom_llm_provider.as_deref(), Some("openai"));
    assert_eq!(request.api_key.as_ref().unwrap().expose_secret(), "sk-1");
}

#[test]
fn request_list_limit_zero_is_kept() {
    assert_eq!(list(r#"{"limit": 0}"#).limit, Some(0));
}

#[test]
fn request_status_absent_timeout_takes_the_request_timeout() {
    assert_eq!(status(r#"{"video_id": "v"}"#).timeout, Some(Duration::ZERO));
}

#[test]
fn request_status_null_timeout_takes_the_request_timeout() {
    assert_eq!(
        status(r#"{"video_id": "v", "timeout": null}"#).timeout,
        Some(Duration::ZERO)
    );
}

#[test]
fn request_status_given_timeout_is_kept() {
    assert_eq!(
        status(r#"{"video_id": "v", "timeout": 12.5}"#).timeout,
        Some(Duration::try_from_secs_f64(12.5).unwrap())
    );
}

#[test]
fn request_drop_params_reads_the_argument_or_the_settings() {
    assert!(drop_params(&wire(r#"{"drop_params": true}"#)).unwrap());
    let payload = r#"{"arguments": {"prompt": "p"}, "settings": {"drop_params": true}}"#;
    let (wire, _media) = parse(payload, &mut []).unwrap();
    assert!(drop_params(&wire).unwrap());
    let disabled = r#"{"arguments": {"prompt": "p"}, "settings": {"drop_params": false}}"#;
    let (disabled_wire, _disabled_media) = parse(disabled, &mut []).unwrap();
    assert!(!drop_params(&disabled_wire).unwrap());
}

#[test]
fn error_generation_without_a_prompt_names_the_field() {
    let error = build(Operation::Generate, "{}").err().unwrap();
    assert!(matches!(error, ConvertError::Field { .. }));
    assert_eq!(error.to_string(), "prompt must be a string");
}

#[test]
fn error_generation_seconds_of_the_wrong_type_names_the_field() {
    let error = build(Operation::Generate, r#"{"prompt": "p", "seconds": true}"#)
        .err()
        .unwrap();
    assert_eq!(error.to_string(), "seconds must be a string");
}

#[test]
fn error_timeout_that_is_not_a_number_names_the_field() {
    let error = build(
        Operation::Content,
        r#"{"video_id": "v", "timeout": "soon"}"#,
    )
    .err()
    .unwrap();
    assert_eq!(error.to_string(), "timeout must be a number of seconds");
}

#[test]
fn error_list_limit_that_is_not_an_integer_names_the_field() {
    let error = build(Operation::List, r#"{"limit": 1.5}"#).err().unwrap();
    assert_eq!(error.to_string(), "limit must be an integer");
}

#[test]
fn error_header_value_that_is_not_a_string_names_the_field() {
    let error = build(
        Operation::Remix,
        r#"{"video_id": "v", "prompt": "p", "extra_headers": {"x-one": 1}}"#,
    )
    .err()
    .unwrap();
    assert_eq!(error.to_string(), "extra_headers must be a map of strings");
}

#[test]
fn error_input_reference_with_an_unknown_blob_is_reported() {
    let mut blobs = Vec::new();
    let payload = r#"{"arguments": {"prompt": "p", "input_reference": {"$file": {"blob": 0}}}}"#;
    let error = parse(payload, &mut blobs).err().unwrap();
    assert_eq!(
        error.to_string(),
        "the input_reference file names blob 0, which the call did not send"
    );
}

#[test]
fn error_payload_that_is_not_json_is_reported() {
    let error = parse("not json", &mut []).err().unwrap();
    assert!(
        error
            .to_string()
            .starts_with("the native request text is not JSON of the expected shape:")
    );
}

#[test]
fn request_adapter_emits_openai_video_headers_and_urls() {
    let config = OpenAiVideoConfig {
        environment: BTreeMap::from([("OPENAI_API_KEY".to_owned(), "env-key".to_owned())]),
        global_api_key: None,
        openai_key: None,
        reference: None,
    };
    let params = Params::new();
    let given = IndexMap::from([("x-custom".to_owned(), "value".to_owned())]);
    let headers = VideoHttpConfig::validate_environment(
        &config,
        VideoEnvironment {
            headers: &given,
            model: "openai/sora-2",
            api_key: None,
            litellm_params: None,
        },
    );
    assert_eq!(headers.get("x-custom").map(String::as_str), Some("value"));
    assert_eq!(
        headers.get("Authorization").map(String::as_str),
        Some("Bearer env-key")
    );
    assert_eq!(
        VideoHttpConfig::get_complete_url(&config, "m", Some("https://api.example/v1"), &params)
            .unwrap(),
        "https://api.example/v1/videos"
    );
    assert!(config.use_multipart_form_data());
}

#[test]
fn request_adapter_transforms_generation_content_and_bytes() {
    let config = OpenAiVideoConfig {
        environment: BTreeMap::new(),
        global_api_key: None,
        openai_key: None,
        reference: Some(Reference {
            name: Some("source.png".to_owned()),
            content: b"png-data".to_vec(),
        }),
    };
    let params = Params::new();
    let headers = IndexMap::new();
    let context = VideoRequestContext {
        api_base: "https://api.example/v1/videos",
        litellm_params: &params,
        headers: &headers,
    };
    let (fields, files, base) = config
        .transform_video_create_request("sora-2", "prompt", &params, context)
        .unwrap();
    assert_eq!(base, "https://api.example/v1/videos");
    assert_eq!(fields.get("model"), Some(&Json::Text("sora-2".to_owned())));
    assert_eq!(fields.get("prompt"), Some(&Json::Text("prompt".to_owned())));
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].field, "input_reference");
    assert_eq!(files[0].filename.as_deref(), Some("source.png"));
    assert_eq!(files[0].content, b"png-data");
}

#[test]
fn request_adapter_transforms_content_and_bytes() {
    let config = OpenAiVideoConfig {
        environment: BTreeMap::new(),
        global_api_key: None,
        openai_key: None,
        reference: None,
    };
    let params = Params::new();
    let headers = IndexMap::new();
    let context = VideoRequestContext {
        api_base: "https://api.example/v1/videos",
        litellm_params: &params,
        headers: &headers,
    };
    let (url, query) = config
        .transform_video_content_request("video_123", Some("thumbnail"), context)
        .unwrap();
    assert_eq!(
        url,
        "https://api.example/v1/videos/video_123/content?variant=thumbnail"
    );
    assert!(query.is_empty());
    let upstream = UpstreamResponse {
        status: 200,
        headers: http::HeaderMap::new(),
        body: b"video-bytes".to_vec(),
    };
    assert_eq!(
        config
            .transform_video_content_response(&upstream, &mut VideoLogging::default())
            .unwrap(),
        b"video-bytes"
    );
}

#[test]
fn request_adapter_maps_generation_options_without_dropping_values() {
    let config = OpenAiVideoConfig {
        environment: BTreeMap::new(),
        global_api_key: None,
        openai_key: None,
        reference: None,
    };
    let options = VideoCreateOptionalRequestParams {
        input_reference: None,
        image: Some(Json::Text("image".to_owned())),
        parameters: Some(IndexMap::from([(
            "quality".to_owned(),
            Json::Text("high".to_owned()),
        )])),
        model: Some("sora-2".to_owned()),
        resolution: Some("720p".to_owned()),
        seconds: Some("8".to_owned()),
        size: Some("portrait".to_owned()),
        characters: None,
        user: Some("user-1".to_owned()),
        extra_headers: Some(IndexMap::from([("x-one".to_owned(), "one".to_owned())])),
        extra_body: Some(IndexMap::from([("watermark".to_owned(), "yes".to_owned())])),
    };
    let mapped = VideoGenerationConfig::map_openai_params(&config, &options, "model", false);
    assert_eq!(mapped.get("model"), Some(&Json::Text("sora-2".to_owned())));
    assert_eq!(
        mapped.get("resolution"),
        Some(&Json::Text("720p".to_owned()))
    );
    assert_eq!(mapped.get("seconds"), Some(&Json::Text("8".to_owned())));
    assert_eq!(mapped.get("size"), Some(&Json::Text("portrait".to_owned())));
    assert_eq!(mapped.get("user"), Some(&Json::Text("user-1".to_owned())));
    assert_eq!(mapped.get("image"), Some(&Json::Text("image".to_owned())));
    assert!(
        matches!(mapped.get("extra_body"), Some(Json::Object(values)) if values.get("watermark") == Some(&Json::Text("yes".to_owned())))
    );
}

#[test]
fn request_adapter_builds_status_urls() {
    let config = OpenAiVideoConfig {
        environment: BTreeMap::new(),
        global_api_key: None,
        openai_key: None,
        reference: None,
    };
    let params = Params::new();
    let headers = IndexMap::new();
    let context = VideoRequestContext {
        api_base: "https://api.example/v1/videos",
        litellm_params: &params,
        headers: &headers,
    };
    let (status_url, status_params) = VideoStatusConfig::transform_video_status_retrieve_request(
        &config,
        "video_123",
        StatusContext {
            api_base: context.api_base,
            litellm_params: context.litellm_params,
            headers: context.headers,
        },
    )
    .unwrap();
    assert_eq!(status_url, "https://api.example/v1/videos/video_123");
    assert!(status_params.is_empty());
}

#[test]
fn request_adapter_builds_remix_urls() {
    let config = OpenAiVideoConfig {
        environment: BTreeMap::new(),
        global_api_key: None,
        openai_key: None,
        reference: None,
    };
    let params = Params::new();
    let headers = IndexMap::new();
    let context = VideoRequestContext {
        api_base: "https://api.example/v1/videos",
        litellm_params: &params,
        headers: &headers,
    };
    let (remix_url, remix_params) = VideoRemixListConfig::transform_video_remix_request(
        &config,
        "video_123",
        "remix prompt",
        None,
        RemixContext {
            api_base: context.api_base,
            litellm_params: context.litellm_params,
            headers: context.headers,
        },
    )
    .unwrap();
    assert_eq!(remix_url, "https://api.example/v1/videos/video_123/remix");
    assert!(
        matches!(remix_params.get("prompt"), Some(Json::Text(value)) if value == "remix prompt")
    );
}

#[test]
fn request_adapter_builds_list_urls() {
    let config = OpenAiVideoConfig {
        environment: BTreeMap::new(),
        global_api_key: None,
        openai_key: None,
        reference: None,
    };
    let params = Params::new();
    let headers = IndexMap::new();
    let context = VideoRequestContext {
        api_base: "https://api.example/v1/videos",
        litellm_params: &params,
        headers: &headers,
    };
    let (list_url, list_params) = VideoRemixListConfig::transform_video_list_request(
        &config,
        VideoListQuery {
            after: Some("video_1"),
            limit: Some(3),
            order: Some("asc"),
            extra_query: None,
        },
        RemixContext {
            api_base: context.api_base,
            litellm_params: context.litellm_params,
            headers: context.headers,
        },
    )
    .unwrap();
    assert_eq!(list_url, "https://api.example/v1/videos");
    assert!(matches!(list_params.get("after"), Some(Json::Text(value)) if value == "video_1"));
    assert!(
        matches!(list_params.get("limit"), Some(Json::Number(value)) if value.as_i64() == Some(3))
    );
    assert!(matches!(list_params.get("order"), Some(Json::Text(value)) if value == "asc"));
}

#[test]
fn request_video_session_uses_the_supplied_environment() {
    let session = VideoSession::build(
        r#"{"environment":{"OPENAI_API_KEY":"test-key","REQUEST_TIMEOUT":"42"}}"#,
    )
    .unwrap();
    assert_eq!(
        session
            .environment
            .get("OPENAI_API_KEY")
            .map(String::as_str),
        Some("test-key")
    );
    assert!((session.request_timeout_seconds - 42.0_f64).abs() < f64::EPSILON);
    assert!(session.known_model_names.is_empty());
    assert!(session.custom_providers.is_empty());
    let resolution = session.resolution();
    assert_eq!(
        resolution
            .environment
            .get("OPENAI_API_KEY")
            .map(String::as_str),
        Some("test-key")
    );
    assert!(resolution.known_model_names.is_empty());
    assert!(resolution.custom_providers.is_empty());
    let config = session_config(&session, &SettingsWire::default(), None);
    let headers = config.headers(&IndexMap::new(), None, None);
    assert_eq!(
        headers.get("Authorization").map(String::as_str),
        Some("Bearer test-key")
    );
}

#[test]
fn request_empty_reference_name() {
    let file = CallFile {
        name: String::new(),
        bytes: b"png".to_vec(),
        content_type: None,
    };
    assert_eq!(super::reference(&file).name, None);
}

#[test]
fn request_drop_params_defaults() {
    assert!(!drop_params(&wire("{}")).unwrap());
    assert!(!drop_params(&wire(r#"{"drop_params":null}"#)).unwrap());
    assert!(!drop_params(&wire(r#"{"drop_params":false}"#)).unwrap());
}
