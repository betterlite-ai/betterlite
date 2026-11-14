use std::future::{Future, ready};

use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::media::inline::RemoteMediaFetcher;
use betterlite_prompts::media_fetch::error::ImageFetchError;
use betterlite_prompts::message_content::Message;
use rstest::rstest;
use tokio::runtime::{Builder, Runtime};

use crate::gemini_options::params::ReasoningSupport;
use crate::studio_chat::input::{ImageUrlFields, image_url_fields};
use crate::studio_chat::settings::StudioSettings;
use crate::studio_chat::{SUPPORTED_OPENAI_PARAMS, supported_openai_params, transform_messages};

struct NoFetch;

impl RemoteMediaFetcher for NoFetch {
    fn convert_url(&self, url: &str) -> impl Future<Output = Result<String, ImageFetchError>> {
        ready(Err(ImageFetchError::Attempts {
            url: url.to_owned(),
        }))
    }

    fn convert_urls(
        &self,
        urls: &[String],
    ) -> impl Future<Output = Result<Vec<String>, ImageFetchError>> {
        let url = urls.first().cloned().unwrap_or_default();
        ready(Err(ImageFetchError::Attempts { url }))
    }
}

struct Catalog(&'static [&'static str]);

impl ReasoningSupport for Catalog {
    fn supports_reasoning(&self, model: &str) -> bool {
        self.0.contains(&model)
    }
}

fn no_reasoning() -> Catalog {
    Catalog(&[])
}

fn reasoning_models() -> Catalog {
    Catalog(&["gemini-2.5-pro"])
}

#[test]
fn the_plain_studio_list_omits_the_vertex_only_params() {
    let expected = vec![
        "temperature",
        "top_p",
        "max_tokens",
        "max_completion_tokens",
        "stream",
        "tools",
        "tool_choice",
        "functions",
        "response_format",
        "n",
        "stop",
        "logprobs",
        "frequency_penalty",
        "presence_penalty",
        "seed",
        "modalities",
        "parallel_tool_calls",
        "web_search_options",
        "include_server_side_tool_invocations",
        "service_tier",
    ];

    let params = supported_openai_params("gemini-2.0-flash", &no_reasoning());

    assert_eq!(params, expected);
    assert_eq!(SUPPORTED_OPENAI_PARAMS.len(), 20);
    assert!(!params.contains(&"top_k"));
    assert!(!params.contains(&"top_logprobs"));
    assert!(!params.contains(&"extra_headers"));
}

#[test]
fn reasoning_appends_the_thinking_params_after_the_penalties() {
    let params = supported_openai_params("gemini-2.5-pro", &reasoning_models());

    assert_eq!(
        params.get(18..),
        Some(
            [
                "include_server_side_tool_invocations",
                "service_tier",
                "reasoning_effort",
                "thinking"
            ]
            .as_slice()
        ),
    );
    assert_eq!(params.len(), 22);
}

#[test]
fn gemini_three_advertises_thinking_without_a_catalog_entry() {
    let params = supported_openai_params("gemini-3-pro-preview", &no_reasoning());

    assert!(params.ends_with(&["reasoning_effort", "thinking"]));
    assert_eq!(params.len(), 22);
}

#[test]
fn the_tts_marker_appends_audio_last_and_keeps_reasoning_absent() {
    let params = supported_openai_params("gemini-2.5-flash-preview-tts", &no_reasoning());

    assert_eq!(params.last().copied(), Some("audio"));
    assert_eq!(params.len(), 21);
    assert!(!params.contains(&"reasoning_effort"));
}

#[test]
fn audio_follows_the_thinking_params_for_a_tts_gemini_three_model() {
    let params = supported_openai_params("gemini-3-pro-preview-tts", &no_reasoning());

    assert_eq!(
        params.get(20..),
        Some(["reasoning_effort", "thinking", "audio"].as_slice()),
    );
}

#[test]
fn the_tts_check_is_case_sensitive_and_a_substring() {
    assert_eq!(
        supported_openai_params("gemini-2.5-flash-preview-TTS", &no_reasoning()).len(),
        20
    );
    assert_eq!(
        supported_openai_params("gemini-2.5-flash-preview-tts-001", &no_reasoning()).len(),
        21
    );
}

#[rstest]
#[case(
    r#"{"type": "image_url", "image_url": {"url": "data:image/png;base64,YWJj", "format": "image/gif", "detail": "high"}}"#,
    ImageUrlFields {
        url: Some("data:image/png;base64,YWJj".to_owned()),
        format: Some("image/gif".to_owned()),
        detail: Some("high".to_owned()),
    }
)]
#[case(
    r#"{"type": "image_url", "image_url": "data:image/png;base64,YWJj"}"#,
    ImageUrlFields {
        url: Some("data:image/png;base64,YWJj".to_owned()),
        format: None,
        detail: None,
    }
)]
#[case(r#"{"type": "image_url"}"#, ImageUrlFields::default())]
#[case(
    r#"{"type": "image_url", "image_url": null}"#,
    ImageUrlFields::default()
)]
fn image_url_fields_reads_the_part_the_way_the_studio_prepass_does(
    #[case] source: &str,
    #[case] expected: ImageUrlFields,
) {
    let part: Object = serde_json::from_str(source).unwrap();

    assert_eq!(image_url_fields(&part), expected);
}

fn parse_json(source: &str) -> Json {
    serde_json::from_str(source).unwrap()
}

fn contents(source: &str, model: &str) -> Json {
    let messages: Vec<Message> = serde_json::from_str(source).unwrap();
    let runtime: Runtime = Builder::new_current_thread().build().unwrap();
    let history = runtime
        .block_on(transform_messages(&messages, model, &NoFetch))
        .unwrap();
    let encoded = serde_json::to_value(history.contents).unwrap();
    serde_json::from_value(encoded).unwrap()
}

#[test]
fn inline_image_detail_reaches_the_part_for_gemini_three() {
    let source = r#"[{"role": "user", "content": [
        {"type": "text", "text": "hi"},
        {"type": "image_url", "image_url": {"url": "data:image/png;base64,YWJj", "detail": "high"}}
    ]}]"#;

    assert_eq!(
        contents(source, "gemini-3-pro-preview"),
        parse_json(
            r#"[{"role": "user", "parts": [
                {"text": "hi"},
                {
                    "inline_data": {"data": "YWJj", "mime_type": "image/png"},
                    "media_resolution": {"level": "MEDIA_RESOLUTION_HIGH"}
                }
            ]}]"#
        )
    );
}

#[test]
fn a_files_api_uri_passes_through_as_file_data() {
    let source = r#"[{"role": "user", "content": [
        {"type": "text", "text": "read"},
        {"type": "file", "file": {"file_id": "https://generativelanguage.googleapis.com/v1beta/files/abc123"}}
    ]}]"#;

    assert_eq!(
        contents(source, "gemini-2.5-flash"),
        parse_json(
            r#"[{"role": "user", "parts": [
                {"text": "read"},
                {"file_data": {"file_uri": "https://generativelanguage.googleapis.com/v1beta/files/abc123"}}
            ]}]"#
        )
    );
}

#[test]
fn class_defaults_stay_absent_from_get_config() {
    let config = StudioSettings::default().get_config().unwrap();

    assert!(config.is_empty(), "{config:?}");
}
