#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_core::api::video_remix_list_transport::config::VideoRemixListConfig;
use betterlite_core::api::video_remix_list_transport::records::VideoRequestContext;
use betterlite_core::api::video_status_transport::config::VideoStatusConfig as _;
use betterlite_core::api::video_status_transport::records::VideoRequestContext as StatusContext;
use betterlite_core::api::videos_http_transport::video_config::{
    VideoHttpConfig as _, VideoLogging,
};
use betterlite_http::proxy::UpstreamResponse;
use betterlite_models::decisions::Json;
use http::header::{HeaderMap, HeaderValue};
use indexmap::IndexMap;
use rstest::rstest;

use super::OpenAiVideoConfig;
use crate::video_calls::convert::ConvertError;

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

#[rstest]
#[case::below_success(199, b"provider rejected", "provider rejected")]
#[case::redirect(300, b"provider rejected", "provider rejected")]
#[case::unauthorized(401, b"provider rejected", "provider rejected")]
#[case::missing(404, b"provider rejected", "provider rejected")]
#[case::invalid_utf8(401, b"\xff", "1 bytes that are not UTF-8")]
#[case::unicode(404, "\u{e9}".as_bytes(), "\u{e9}")]
fn response_status_keeps_facts(#[case] status: u16, #[case] body: &[u8], #[case] text: &str) {
    let mut headers = HeaderMap::new();
    headers.insert("x-request-id", HeaderValue::from_bytes(body).unwrap());
    let response = UpstreamResponse {
        status,
        headers,
        body: body.to_vec(),
    };
    let failure = super::support::check_response(&response).unwrap_err();
    assert_eq!(failure.status_code, Some(status));
    assert_eq!(failure.message, text);
    assert_eq!(failure.headers.unwrap().get("x-request-id").unwrap(), text);
    let adapter = config();
    let error = adapter
        .transform_video_content_response(&response, &mut VideoLogging::default())
        .unwrap_err();
    let facts = adapter.error_facts(&error);
    assert_eq!(facts.status_code, Some(status));
    assert_eq!(facts.message, text);
    assert_eq!(facts.headers.get("x-request-id").unwrap(), text);
}

#[rstest]
#[case::first_success(200)]
#[case::last_success(299)]
fn response_success_is_kept(#[case] status: u16) {
    let response = UpstreamResponse {
        status,
        headers: HeaderMap::new(),
        body: b"content".to_vec(),
    };
    super::support::check_response(&response).unwrap();
    assert_eq!(
        config()
            .transform_video_content_response(&response, &mut VideoLogging::default())
            .unwrap(),
        b"content"
    );
}

#[test]
fn plain_error_facts_have_no_status() {
    let error = ConvertError::Field {
        name: "variant".to_owned(),
        expected: "a string",
    };
    let facts = config().error_facts(&error);
    assert_eq!(facts.message, "variant must be a string");
    assert_eq!(facts.status_code, None);
    assert!(facts.headers.is_empty());
}
