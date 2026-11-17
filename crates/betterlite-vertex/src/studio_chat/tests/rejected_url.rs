use std::future::{Future, ready};
use std::sync::atomic::{AtomicUsize, Ordering};

use betterlite_prompts::arguments::json::Json;
use betterlite_prompts::media::inline::RemoteMediaFetcher;
use betterlite_prompts::media_fetch::error::ImageFetchError;
use betterlite_prompts::message_content::Message;
use rstest::rstest;
use tokio::runtime::{Builder, Runtime};

use crate::gemini_history::convert::History;
use crate::gemini_history::error::HistoryError;
use crate::studio_chat::transform_messages;

const FILES_URI: &str = "https://generativelanguage.googleapis.com/v1beta/files/abc123";

#[derive(Default)]
struct CountingFetch(AtomicUsize);

impl RemoteMediaFetcher for CountingFetch {
    fn convert_url(&self, url: &str) -> impl Future<Output = Result<String, ImageFetchError>> {
        self.0.fetch_add(1, Ordering::SeqCst);
        ready(Err(ImageFetchError::Attempts {
            url: url.to_owned(),
        }))
    }

    fn convert_urls(
        &self,
        urls: &[String],
    ) -> impl Future<Output = Result<Vec<String>, ImageFetchError>> {
        self.0.fetch_add(1, Ordering::SeqCst);
        let url = urls.first().cloned().unwrap_or_default();
        ready(Err(ImageFetchError::Attempts { url }))
    }
}

fn parse(source: &str) -> Vec<Message> {
    serde_json::from_str(source).unwrap()
}

fn current_thread_runtime() -> Runtime {
    Builder::new_current_thread().build().unwrap()
}

fn convert(fetcher: &CountingFetch, source: &str, model: &str) -> Result<History, HistoryError> {
    let messages = parse(source);
    let runtime = current_thread_runtime();
    runtime.block_on(transform_messages(&messages, model, fetcher))
}

#[rstest]
#[case::http_image(
    r#"[{"role": "user", "content": [{"type": "image_url", "image_url": {"url": "http://example.com/cat.png"}}]}]"#,
    Some("invalid_request_error"),
    Some("image_url"),
    Some("unsupported_remote_media")
)]
#[case::http_file(
    r#"[{"role": "user", "content": [{"type": "file", "file": {"file_id": "http://example.com/doc.pdf", "format": "application/pdf"}}]}]"#,
    Some("invalid_request_error"),
    Some("file"),
    Some("unsupported_remote_media")
)]
#[case::https_file_without_mime(
    r#"[{"role": "user", "content": [{"type": "file", "file": {"file_id": "https://example.com/doc.pdf"}}]}]"#,
    Some("invalid_request_error"),
    Some("file"),
    Some("unsupported_remote_media")
)]
#[case::https_image_without_mime(
    r#"[{"role": "user", "content": [{"type": "image_url", "image_url": {"url": "https://example.com/cat"}}]}]"#,
    Some("invalid_request_error"),
    Some("image_url"),
    Some("unsupported_remote_media")
)]
fn remote_media_left_unprepared_is_refused_without_a_fetch(
    #[case] source: &str,
    #[case] error_type: Option<&str>,
    #[case] param: Option<&str>,
    #[case] code: Option<&str>,
) {
    let fetcher = CountingFetch::default();

    let error = convert(&fetcher, source, "gemini-2.5-flash").unwrap_err();

    assert_eq!(error.status(), 400);
    assert_eq!(error.error_type(), error_type);
    assert_eq!(error.param(), param);
    assert_eq!(error.code(), code);
    assert_eq!(
        fetcher.0.load(Ordering::SeqCst),
        0,
        "an unprepared part must not reach the transport"
    );
}

#[rstest]
#[case(r#""https://example.com/cat.png""#)]
#[case(r#"{"url": "https://example.com/cat.png", "format": "image/png"}"#)]
fn an_unprepared_https_extension_is_forwarded_as_file_data(#[case] image_url: &str) {
    let fetcher = CountingFetch::default();
    let source = format!(
        r#"[{{"role": "user", "content": [{{"type": "text", "text": "look"}}, {{"type": "image_url", "image_url": {image_url}}}]}}]"#
    );

    let history = convert(&fetcher, &source, "gemini-2.5-flash").unwrap();

    let encoded: Json =
        serde_json::from_value(serde_json::to_value(&history.contents).unwrap()).unwrap();
    let expected: Json = serde_json::from_str(
        r#"[{"role": "user", "parts": [{"text": "look"}, {"file_data": {"mime_type": "image/png", "file_uri": "https://example.com/cat.png"}}]}]"#,
    )
    .unwrap();
    assert_eq!(encoded, expected);
    assert_eq!(fetcher.0.load(Ordering::SeqCst), 0);
}

#[rstest]
#[case(r#"[{"role": "user", "content": [{"type": "file"}]}]"#)]
#[case(r#"[{"role": "user", "content": [{"type": "file", "file": null}]}]"#)]
#[case(r#"[{"role": "user", "content": [{"type": "file", "file": "not-an-object"}]}]"#)]
fn a_file_block_without_an_object_file_field_is_a_bodyless_bad_request(#[case] source: &str) {
    let fetcher = CountingFetch::default();

    let error = convert(&fetcher, source, "gemini-2.5-flash").unwrap_err();

    assert_eq!(error.status(), 400);
    assert_eq!(error.error_type(), None);
    assert_eq!(error.param(), None);
    assert_eq!(error.code(), Some("400"));
    assert_eq!(fetcher.0.load(Ordering::SeqCst), 0);
}

#[rstest]
#[case(r#"[{"role": "user", "content": [{"type": "image_url", "image_url": "data:image/png;base64,YWJj"}]}]"#)]
#[case(r#"[{"role": "user", "content": [{"type": "image_url", "image_url": {"url": "https://generativelanguage.googleapis.com/v1beta/files/abc123"}}]}]"#)]
#[case(r#"[{"role": "user", "content": [{"type": "file", "file": {"file_id": "https://generativelanguage.googleapis.com/v1beta/files/abc123"}}]}]"#)]
#[case(r#"[{"role": "user", "content": [{"type": "file", "file": {"file_id": "gs://bucket/doc.pdf", "format": "application/pdf"}}]}]"#)]
fn inline_data_and_uploaded_uris_pass_without_a_fetch(#[case] source: &str) {
    let fetcher = CountingFetch::default();

    let history = convert(&fetcher, source, "gemini-2.5-flash").unwrap();

    assert_eq!(history.contents.len(), 1);
    assert_eq!(fetcher.0.load(Ordering::SeqCst), 0);
}

#[test]
fn a_files_api_uri_is_sent_as_file_data() {
    let fetcher = CountingFetch::default();
    let source = format!(
        r#"[{{"role": "user", "content": [{{"type": "text", "text": "read"}}, {{"type": "file", "file": {{"file_id": "{FILES_URI}"}}}}]}}]"#
    );

    let history = convert(&fetcher, &source, "gemini-2.5-flash").unwrap();

    let encoded: Json =
        serde_json::from_value(serde_json::to_value(&history.contents).unwrap()).unwrap();
    let expected: Json = serde_json::from_str(&format!(
        r#"[{{"role": "user", "parts": [{{"text": "read"}}, {{"file_data": {{"file_uri": "{FILES_URI}"}}}}]}}]"#
    ))
    .unwrap();
    assert_eq!(encoded, expected);
}
