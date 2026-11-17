#![cfg(test)]

use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::Mutex;

use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::media::inline::RemoteMediaFetcher;
use betterlite_prompts::media_fetch::error::ImageFetchError;
use rstest::rstest;
use serde_json::{Number, Value};
use tokio::runtime::{Builder, Runtime};

use super::{
    ChatContent, ChatContentError, FetchedMedia, Mode, RefuseDownloads, TransformedRequest,
    UrlDownloader,
};
use crate::chat::ChatMessage;

const CONVERT_URLS: &str = "<convert_urls>";
const DATA_URL: &str = "data:application/pdf;base64,JVBERi0=";
const REPORT: &str = "https://example.com/report.pdf";

fn runtime() -> Runtime {
    Builder::new_current_thread().build().unwrap()
}

fn object(pairs: Vec<(&str, Value)>) -> Value {
    Value::Object(
        pairs
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect(),
    )
}

struct MapFetcher<'urls> {
    urls: &'urls BTreeMap<String, String>,
    calls: Mutex<Vec<String>>,
}

impl<'urls> MapFetcher<'urls> {
    fn new(urls: &'urls BTreeMap<String, String>) -> Self {
        Self {
            urls,
            calls: Mutex::new(Vec::new()),
        }
    }

    fn record(&self, url: &str) {
        self.calls.lock().unwrap().push(url.to_owned());
    }

    fn answer(&self, url: &str) -> Result<String, ImageFetchError> {
        self.record(url);
        self.urls
            .get(url)
            .cloned()
            .ok_or_else(|| ImageFetchError::Attempts {
                url: url.to_owned(),
            })
    }

    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

impl RemoteMediaFetcher for MapFetcher<'_> {
    fn convert_url(&self, url: &str) -> impl Future<Output = Result<String, ImageFetchError>> {
        ready(self.answer(url))
    }

    // LiteLLM awaits one conversion per content item, so a batch call is a regression every test must fail on
    fn convert_urls(
        &self,
        urls: &[String],
    ) -> impl Future<Output = Result<Vec<String>, ImageFetchError>> {
        self.record(CONVERT_URLS);
        ready(urls.first().map_or_else(
            || Ok(Vec::new()),
            |url| Err(ImageFetchError::Attempts { url: url.clone() }),
        ))
    }
}

struct Fixture {
    model: String,
    messages: Vec<ChatMessage>,
    optional_params: Object,
    environment: BTreeMap<String, String>,
}

impl Fixture {
    fn new(messages: &Value) -> Self {
        Self {
            model: "gpt-4o".to_owned(),
            messages: serde_json::from_value(messages.clone()).unwrap(),
            optional_params: Object::new(),
            environment: BTreeMap::new(),
        }
    }

    fn content<'borrow>(
        &'borrow self,
        downloads: &'borrow dyn UrlDownloader,
    ) -> ChatContent<'borrow> {
        ChatContent {
            model: &self.model,
            messages: &self.messages,
            optional_params: &self.optional_params,
            provider: Some("openai"),
            api_base: None,
            global_api_base: None,
            openai_system_messages_first: false,
            environment: &self.environment,
            downloads,
        }
    }

    fn fetch_media(&self, fetcher: &MapFetcher<'_>) -> Result<FetchedMedia, ChatContentError> {
        runtime().block_on(self.content(&RefuseDownloads).fetch_media(fetcher))
    }

    fn transform_with_media(
        &self,
        mode: Mode,
        fetcher: &MapFetcher<'_>,
    ) -> Result<TransformedRequest, ChatContentError> {
        runtime().block_on(
            self.content(&RefuseDownloads)
                .transform_with_media(mode, fetcher),
        )
    }

    fn body(&self, mode: Mode, fetcher: &MapFetcher<'_>) -> Value {
        let request = self.transform_with_media(mode, fetcher).unwrap();
        serde_json::to_value(request.to_body().unwrap()).unwrap()
    }

    fn plain_body(&self, mode: Mode, media: &FetchedMedia) -> Value {
        let content = self.content(media);
        let request = content.transform(mode).unwrap();
        serde_json::to_value(request.to_body().unwrap()).unwrap()
    }
}

fn user_content(items: &Value) -> Value {
    Value::Array(vec![object(vec![
        ("role", Value::from("user")),
        ("content", items.clone()),
    ])])
}

fn file_item(file_id: &str) -> Value {
    object(vec![
        ("type", Value::from("file")),
        ("file", object(vec![("file_id", Value::from(file_id))])),
    ])
}

fn image_url_part(url: &str) -> Value {
    object(vec![
        ("type", Value::from("image_url")),
        ("image_url", Value::from(url)),
    ])
}

fn image_url_descriptor(url: &str, detail: &str) -> Value {
    let descriptor = object(vec![
        ("url", Value::from(url)),
        ("format", Value::from("png")),
        ("detail", Value::from(detail)),
    ]);
    object(vec![
        ("type", Value::from("image_url")),
        ("image_url", descriptor),
    ])
}

fn file_with_data(data: &str) -> Value {
    object(vec![
        ("type", Value::from("file")),
        ("file", object(vec![("file_data", Value::from(data))])),
    ])
}

fn url_map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(url, data_url)| ((*url).to_owned(), (*data_url).to_owned()))
        .collect()
}

fn content_part(body: &Value) -> Value {
    body["messages"][0]["content"][0].clone()
}

#[test]
fn fetches_pdf_url_for_user_file_part() {
    let source = user_content(&Value::Array(vec![file_item(REPORT)]));
    let urls = url_map(&[(REPORT, DATA_URL)]);
    let fetcher = MapFetcher::new(&urls);
    let fixture = Fixture::new(&source);
    let body = fixture.body(Mode::Sync, &fetcher);
    assert_eq!(fetcher.calls(), [REPORT]);
    assert_eq!(
        content_part(&body),
        object(vec![
            ("type", Value::from("file")),
            (
                "file",
                object(vec![
                    ("file_data", Value::from(DATA_URL)),
                    ("filename", Value::from("my_file.pdf")),
                ]),
            ),
        ])
    );
    assert_eq!(serde_json::to_value(&fixture.messages).unwrap(), source);
}

#[test]
fn fetches_in_content_item_order() {
    let first = "https://example.com/z.pdf";
    let second = "https://example.com/a.pdf";
    let second_data = "data:application/pdf;base64,QQ==";
    let source = user_content(&Value::Array(vec![file_item(first), file_item(second)]));
    let urls = url_map(&[(first, DATA_URL), (second, second_data)]);
    let fetcher = MapFetcher::new(&urls);
    let body = Fixture::new(&source).body(Mode::Sync, &fetcher);
    assert_eq!(fetcher.calls(), [first, second]);
    let content = body["messages"][0]["content"].clone();
    assert_eq!(content[0]["file"]["file_data"], DATA_URL);
    assert_eq!(content[1]["file"]["file_data"], second_data);
}

#[test]
fn stops_at_the_first_failing_url() {
    let first = "https://example.com/z.pdf";
    let second = "https://example.com/a.pdf";
    let source = user_content(&Value::Array(vec![file_item(first), file_item(second)]));
    let urls = url_map(&[(second, DATA_URL)]);
    let fetcher = MapFetcher::new(&urls);
    let error = Fixture::new(&source)
        .transform_with_media(Mode::Sync, &fetcher)
        .unwrap_err();
    assert_eq!(fetcher.calls(), [first]);
    assert_eq!(error.status_code(), 400);
    assert!(error.to_string().contains("after 3 attempts"));
}

#[test]
fn fetches_for_http_and_www_prefixes() {
    assert_eq!(
        fetched_file("https://example.com/report")["file"]["file_data"],
        DATA_URL
    );
    assert_eq!(
        fetched_file("http://example.com/report")["file"]["file_data"],
        DATA_URL
    );
    assert_eq!(
        fetched_file("www.example.com/report")["file"]["file_data"],
        DATA_URL
    );
}

fn fetched_file(file_id: &str) -> Value {
    let source = user_content(&Value::Array(vec![file_item(file_id)]));
    let urls = url_map(&[(file_id, DATA_URL)]);
    let fetcher = MapFetcher::new(&urls);
    let body = Fixture::new(&source).body(Mode::Sync, &fetcher);
    assert_eq!(fetcher.calls(), [file_id]);
    content_part(&body)
}

#[rstest]
#[case::sync(Mode::Sync)]
#[case::async_base_class(Mode::AsyncBaseClass)]
#[case::async_subclass(Mode::AsyncSubclass)]
fn fetches_for_every_mode(#[case] mode: Mode) {
    let source = user_content(&Value::Array(vec![file_item(REPORT)]));
    let urls = url_map(&[(REPORT, DATA_URL)]);
    let fetcher = MapFetcher::new(&urls);
    let mut fixture = Fixture::new(&source);
    fixture
        .optional_params
        .insert("max_retries".to_owned(), Json::Number(Number::from(3_u64)));
    let body = fixture.body(mode, &fetcher);
    assert_eq!(fetcher.calls(), [REPORT]);
    assert_eq!(content_part(&body)["file"]["file_data"], DATA_URL);
    assert_eq!(
        body.get("max_retries").is_some(),
        mode == Mode::AsyncBaseClass
    );
}

#[test]
fn refusing_fetcher_returns_the_disabled_error() {
    let source = user_content(&Value::Array(vec![file_item(REPORT)]));
    let fixture = Fixture::new(&source);
    let error = runtime()
        .block_on(
            fixture
                .content(&RefuseDownloads)
                .transform_with_media(Mode::Sync, &RefuseDownloads),
        )
        .unwrap_err();
    assert_eq!(error.status_code(), 400);
    assert!(
        error
            .to_string()
            .contains("MAX_IMAGE_URL_DOWNLOAD_SIZE_MB=0")
    );
}

#[test]
fn missing_url_returns_the_fetch_error() {
    let source = user_content(&Value::Array(vec![file_item(REPORT)]));
    let urls = BTreeMap::new();
    let fetcher = MapFetcher::new(&urls);
    let error = Fixture::new(&source)
        .transform_with_media(Mode::Sync, &fetcher)
        .unwrap_err();
    assert_eq!(fetcher.calls(), [REPORT]);
    assert_eq!(error.status_code(), 400);
    assert!(error.to_string().contains("after 3 attempts"));
}

fn untouched_parts() -> Vec<Value> {
    vec![
        image_url_part("https://example.com/cat.png"),
        image_url_descriptor("https://example.com/dog.png", "high"),
        file_item("data:application/pdf;base64,JVBERi0="),
        file_with_data("data:text/plain;base64,aGk="),
        object(vec![
            ("type", Value::from("file")),
            ("file", object(Vec::new())),
        ]),
    ]
}

#[test]
fn non_pdf_parts_never_fetch() {
    let source = user_content(&Value::Array(untouched_parts()));
    let urls = BTreeMap::new();
    let fetcher = MapFetcher::new(&urls);
    let body = Fixture::new(&source).body(Mode::Sync, &fetcher);
    assert!(fetcher.calls().is_empty());
    let content = body["messages"][0]["content"].clone();
    assert_eq!(
        content[0]["image_url"]["url"],
        "https://example.com/cat.png"
    );
    assert_eq!(
        content[1]["image_url"],
        object(vec![
            ("url", Value::from("https://example.com/dog.png")),
            ("detail", Value::from("high")),
        ])
    );
    assert_eq!(
        content[2]["file"]["file_id"],
        "data:application/pdf;base64,JVBERi0="
    );
    assert_eq!(
        content[3]["file"],
        object(vec![
            ("file_data", Value::from("data:text/plain;base64,aGk=")),
            ("filename", Value::from("my_file.pdf")),
        ])
    );
    assert_eq!(content[4]["file"], object(Vec::new()));
}

#[test]
fn other_roles_never_fetch() {
    let source = Value::Array(vec![
        object(vec![
            ("role", Value::from("system")),
            ("content", Value::Array(vec![file_item(REPORT)])),
        ]),
        object(vec![
            ("role", Value::from("assistant")),
            (
                "content",
                Value::Array(vec![
                    object(vec![
                        ("type", Value::from("text")),
                        ("text", Value::from("ok")),
                    ]),
                    file_item("http://example.com/report.pdf"),
                ]),
            ),
        ]),
    ]);
    let urls = url_map(&[(REPORT, DATA_URL)]);
    let fetcher = MapFetcher::new(&urls);
    let body = Fixture::new(&source).body(Mode::Sync, &fetcher);
    assert!(fetcher.calls().is_empty());
    assert_eq!(body["messages"][0]["content"][0]["file"]["file_id"], REPORT);
    assert_eq!(
        body["messages"][1]["content"][1]["file"]["file_id"],
        "http://example.com/report.pdf"
    );
}

#[test]
fn missing_file_after_url_fetches_first() {
    let source = user_content(&Value::Array(vec![
        file_item(REPORT),
        object(vec![("type", Value::from("file"))]),
    ]));
    let urls = url_map(&[(REPORT, DATA_URL)]);
    let fetcher = MapFetcher::new(&urls);
    let error = Fixture::new(&source)
        .transform_with_media(Mode::Sync, &fetcher)
        .unwrap_err();
    assert_eq!(fetcher.calls(), [REPORT]);
    assert_eq!(error.status_code(), 400);
    assert!(
        error
            .to_string()
            .contains("missing the required 'file' field")
    );
    assert!(matches!(error, ChatContentError::MissingFile(_)));
}

#[test]
fn fetched_media_serves_the_plain_transform() {
    let source = user_content(&Value::Array(vec![
        file_item(REPORT),
        object(vec![
            ("type", Value::from("text")),
            ("text", Value::from("look")),
        ]),
    ]));
    let urls = url_map(&[(REPORT, DATA_URL)]);
    let fetcher = MapFetcher::new(&urls);
    let fixture = Fixture::new(&source);
    let media = fixture.fetch_media(&fetcher).unwrap();
    let direct = fixture.body(Mode::Sync, &fetcher);
    let replayed = fixture.plain_body(Mode::Sync, &media);
    assert_eq!(replayed, direct);
    assert_eq!(content_part(&replayed)["file"]["file_data"], DATA_URL);
    assert!(matches!(
        media.download("https://other.example/report.pdf"),
        Err(ImageFetchError::Disabled { .. })
    ));
}
