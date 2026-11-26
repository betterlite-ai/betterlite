#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_core::api::video_remix_list_transport::config::VideoRemixListConfig;
use betterlite_core::api::video_remix_list_transport::records::VideoRequestContext;
use betterlite_core::api::video_status_transport::config::VideoStatusConfig as _;
use betterlite_core::api::video_status_transport::records::VideoRequestContext as StatusContext;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::OpenAiVideoConfig;

fn config() -> OpenAiVideoConfig {
    OpenAiVideoConfig {
        environment: BTreeMap::new(),
        global_api_key: None,
        openai_key: None,
        reference: None,
    }
}

#[test]
fn params_supply_api_key() {
    let params = IndexMap::from([("api_key".to_owned(), Json::Text("params-key".to_owned()))]);
    let headers = config().headers(&IndexMap::new(), None, Some(&params));
    assert_eq!(
        headers.get("Authorization").map(String::as_str),
        Some("Bearer params-key")
    );
}

#[test]
fn remix_extra_body() {
    let extra = IndexMap::from([("seconds".to_owned(), Json::Text("8".to_owned()))]);
    let (url, body) = config()
        .transform_video_remix_request(
            "video_1",
            "prompt",
            Some(&extra),
            VideoRequestContext {
                api_base: "https://example.test/v1/videos",
                headers: &IndexMap::new(),
                litellm_params: &IndexMap::new(),
            },
        )
        .unwrap();
    assert_eq!(url, "https://example.test/v1/videos/video_1/remix");
    assert_eq!(body.get("seconds"), Some(&Json::Text("8".to_owned())));
}

#[test]
fn error_headers_keep_text() {
    let headers = BTreeMap::from([
        ("retry-after".to_owned(), Json::Text("7".to_owned())),
        ("not-text".to_owned(), Json::Bool(true)),
    ]);
    let error = VideoRemixListConfig::get_error_class(&config(), "slow down", 429, headers);
    assert_eq!(error.status_code, 429);
    assert_eq!(error.message, "slow down");
    assert_eq!(
        error.headers.unwrap(),
        vec![("retry-after".to_owned(), "7".to_owned())]
    );
}

#[test]
fn transform_errors_keep_message() {
    let headers = IndexMap::new();
    let params = IndexMap::new();
    let error = config()
        .transform_video_status_retrieve_request(
            "..",
            StatusContext {
                api_base: "https://example.test/v1/videos",
                headers: &headers,
                litellm_params: &params,
            },
        )
        .unwrap_err();
    assert!(!error.message.is_empty());
    assert_eq!(error.status_code, None);
    let failed = config()
        .transform_video_remix_request(
            "..",
            "p",
            None,
            VideoRequestContext {
                api_base: "https://example.test/v1/videos",
                headers: &headers,
                litellm_params: &params,
            },
        )
        .unwrap_err();
    assert!(!failed.source.to_string().is_empty());
}
