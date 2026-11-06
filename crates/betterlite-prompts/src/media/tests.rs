#![cfg(test)]

use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::hint::black_box;
use std::path::PathBuf;
use std::sync::Mutex;
use std::{fs, str};

use rstest::rstest;

use super::base64_string::{get_base64_str, is_base64_encoded};
use super::data_url::{MAX_MEDIA_BYTES, decode_data_url, is_canonical_base64};
use super::error::MediaError;
use super::inline::{
    RemoteMediaFetcher, async_inline_remote_media, inline_remote_media, remote_urls_to_inline,
};
use super::parts::{INLINE_EVERY_REMOTE_URL, INLINE_REMOTE_IMAGE_URLS, RemoteMedia};
use crate::arguments::json::Json;
use crate::media_fetch::error::ImageFetchError;
use crate::message_content::Message;

type Filter = fn(&RemoteMedia) -> bool;

struct InlineCase {
    filter: Filter,
    concurrent: bool,
    messages: &'static str,
    data_urls: &'static [(&'static str, &'static str)],
    expected: &'static str,
    urls: &'static [&'static str],
}

enum Scenario {
    Inline(InlineCase),
    Urls {
        filter: Filter,
        messages: &'static str,
        expected: &'static [&'static str],
    },
    Failure {
        concurrent: bool,
        messages: &'static str,
        data_urls: &'static [(&'static str, &'static str)],
        missing: &'static str,
        converted: &'static [&'static str],
    },
    Decode {
        url: &'static str,
        limit: usize,
        failure: DecodeFailure,
    },
    DecodeOk {
        url: &'static str,
        limit: usize,
        expected: &'static [u8],
    },
}

#[derive(Clone, Copy)]
enum DecodeFailure {
    MissingPrefix,
    MissingMarker,
    TooLarge,
    InvalidBase64,
}

struct MapSource {
    data_urls: BTreeMap<String, String>,
    conversions: Mutex<Vec<String>>,
    batches: Mutex<Vec<Vec<String>>>,
}

impl MapSource {
    fn new(data_urls: BTreeMap<String, String>) -> Self {
        Self {
            data_urls,
            conversions: Mutex::new(Vec::new()),
            batches: Mutex::new(Vec::new()),
        }
    }

    fn result_for(&self, url: &str) -> Result<String, ImageFetchError> {
        self.data_urls.get(url).map_or_else(
            || {
                Err(ImageFetchError::Rejected {
                    url: url.to_owned(),
                })
            },
            |data_url| Ok(data_url.clone()),
        )
    }
}

impl RemoteMediaFetcher for MapSource {
    fn convert_url(&self, url: &str) -> impl Future<Output = Result<String, ImageFetchError>> {
        self.conversions.lock().unwrap().push(url.to_owned());
        ready(self.result_for(url))
    }

    fn convert_urls(
        &self,
        urls: &[String],
    ) -> impl Future<Output = Result<Vec<String>, ImageFetchError>> {
        self.batches.lock().unwrap().push(urls.to_vec());
        let results = urls.iter().map(|url| self.result_for(url)).collect();
        ready(results)
    }
}

fn parsed(source: &str) -> Json {
    serde_json::from_str(source).unwrap()
}

fn as_json<T>(value: &T) -> Json
where
    T: serde::Serialize,
{
    serde_json::from_value(serde_json::to_value(value).unwrap()).unwrap()
}

fn messages_of(source: &str) -> Vec<Message> {
    serde_json::from_str(source).unwrap()
}

fn data_urls_of(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(url, data_url)| ((*url).to_owned(), (*data_url).to_owned()))
        .collect()
}

fn owned(urls: &[&str]) -> Vec<String> {
    urls.iter().map(|url| (*url).to_owned()).collect()
}

async fn run(scenario: Scenario) {
    match scenario {
        Scenario::Inline(case) => run_inline(case).await,
        Scenario::Urls {
            filter,
            messages,
            expected,
        } => run_urls(filter, messages, expected),
        Scenario::Failure {
            concurrent,
            messages,
            data_urls,
            missing,
            converted,
        } => run_failure(concurrent, messages, data_urls, missing, converted).await,
        Scenario::Decode {
            url,
            limit,
            failure,
        } => run_decode(url, limit, failure),
        Scenario::DecodeOk {
            url,
            limit,
            expected,
        } => run_decode_ok(url, limit, expected),
    }
}

async fn run_inline(case: InlineCase) {
    let message_list = messages_of(case.messages);
    let source = MapSource::new(data_urls_of(case.data_urls));
    let inlined = if case.concurrent {
        async_inline_remote_media(&message_list, &case.filter, &source)
            .await
            .unwrap()
    } else {
        inline_remote_media(&message_list, &case.filter, &source)
            .await
            .unwrap()
    };
    assert_eq!(as_json(&inlined), parsed(case.expected));
    assert_eq!(
        remote_urls_to_inline(&message_list, &case.filter),
        owned(case.urls)
    );
    if case.concurrent {
        assert_eq!(source.conversions.lock().unwrap().len(), 0);
        assert_eq!(*source.batches.lock().unwrap(), vec![owned(case.urls)]);
    } else {
        assert_eq!(source.batches.lock().unwrap().len(), 0);
        assert_eq!(*source.conversions.lock().unwrap(), owned(case.urls));
    }
}

fn run_urls(filter: Filter, messages: &str, expected: &[&str]) {
    let message_list = messages_of(messages);
    assert_eq!(
        remote_urls_to_inline(&message_list, &filter),
        owned(expected)
    );
}

async fn run_failure(
    concurrent: bool,
    messages: &str,
    data_urls: &[(&str, &str)],
    missing: &str,
    converted: &[&str],
) {
    let message_list = messages_of(messages);
    let source = MapSource::new(data_urls_of(data_urls));
    let outcome = if concurrent {
        async_inline_remote_media(&message_list, &INLINE_EVERY_REMOTE_URL, &source).await
    } else {
        inline_remote_media(&message_list, &INLINE_EVERY_REMOTE_URL, &source).await
    };
    let error = outcome.unwrap_err();
    assert_eq!(
        error.to_string(),
        ImageFetchError::Rejected {
            url: missing.to_owned()
        }
        .to_string()
    );
    assert!(matches!(
        error,
        MediaError::Fetch(ImageFetchError::Rejected { .. })
    ));
    assert_eq!(*source.conversions.lock().unwrap(), owned(converted));
}

fn run_decode(url: &str, limit: usize, failure: DecodeFailure) {
    let error = decode_data_url(url, limit).unwrap_err();
    let expected = match failure {
        DecodeFailure::MissingPrefix => matches!(error, MediaError::MissingDataPrefix),
        DecodeFailure::MissingMarker => matches!(error, MediaError::MissingBase64Marker),
        DecodeFailure::TooLarge => matches!(error, MediaError::TooLarge { .. }),
        DecodeFailure::InvalidBase64 => matches!(error, MediaError::InvalidBase64(_)),
    };
    assert!(expected, "unexpected error {error}");
}

fn run_decode_ok(url: &str, limit: usize, expected: &[u8]) {
    assert_eq!(decode_data_url(url, limit).unwrap(), expected);
}

#[rstest]
#[case::inlines_image_url_object_and_string(Scenario::Inline(InlineCase {
    filter: INLINE_EVERY_REMOTE_URL,
    concurrent: false,
    messages: r#"[{"role":"user","content":[{"type":"text","text":"hi"},{"type":"image_url","image_url":{"url":"https://a.example/cat.png","detail":"high"}},{"type":"image_url","image_url":"https://a.example/dog.png"}]}]"#,
    data_urls: &[("https://a.example/cat.png", "data:image/png;base64,Y2F0"), ("https://a.example/dog.png", "data:image/png;base64,ZG9n")],
    expected: r#"[{"role":"user","content":[{"type":"text","text":"hi"},{"type":"image_url","image_url":{"url":"data:image/png;base64,Y2F0","detail":"high"}},{"type":"image_url","image_url":"data:image/png;base64,ZG9n"}]}]"#,
    urls: &["https://a.example/cat.png", "https://a.example/dog.png"],
}))]
#[case::inlines_file_part_and_keeps_its_format(Scenario::Inline(InlineCase {
    filter: INLINE_EVERY_REMOTE_URL,
    concurrent: false,
    messages: r#"[{"role":"user","content":[{"type":"file","file":{"file_id":"https://a.example/notes.txt","filename":"notes.txt","format":"text/plain"}}]}]"#,
    data_urls: &[("https://a.example/notes.txt", "data:text/plain;base64,bm90ZXM=")],
    expected: r#"[{"role":"user","content":[{"type":"file","file":{"filename":"notes.txt","format":"text/plain","file_data":"data:text/plain;base64,bm90ZXM="}}]}]"#,
    urls: &["https://a.example/notes.txt"],
}))]
#[case::infers_pdf_format_for_a_file_without_one(Scenario::Inline(InlineCase {
    filter: INLINE_EVERY_REMOTE_URL,
    concurrent: false,
    messages: r#"[{"role":"user","content":[{"type":"file","file":{"file_id":"https://a.example/report.pdf","filename":"report.pdf"}},{"type":"file","file":{"file_id":"https://a.example/other.pdf","format":"text/plain"}}]}]"#,
    data_urls: &[("https://a.example/report.pdf", "data:application/pdf;base64,cGRm"), ("https://a.example/other.pdf", "data:application/pdf;base64,b3RoZXI=")],
    expected: r#"[{"role":"user","content":[{"type":"file","file":{"filename":"report.pdf","format":"application/pdf","file_data":"data:application/pdf;base64,cGRm"}},{"type":"file","file":{"format":"text/plain","file_data":"data:application/pdf;base64,b3RoZXI="}}]}]"#,
    urls: &["https://a.example/report.pdf", "https://a.example/other.pdf"],
}))]
#[case::inlines_document_and_image_sources(Scenario::Inline(InlineCase {
    filter: INLINE_EVERY_REMOTE_URL,
    concurrent: true,
    messages: r#"[{"role":"user","content":[{"type":"document","title":"Report","source":{"type":"url","url":"https://a.example/report.pdf","media_type":"application/pdf"}},{"type":"image","source":{"type":"url","url":"https://a.example/photo.png"}}]}]"#,
    data_urls: &[("https://a.example/report.pdf", "data:foo/bar;base64,cGRm"), ("https://a.example/photo.png", "data:image/png;base64,cGhvdG8=")],
    expected: r#"[{"role":"user","content":[{"type":"document","title":"Report","source":{"type":"base64","media_type":"application/pdf","data":"cGRm"}},{"type":"image","source":{"type":"base64","media_type":"image/png","data":"cGhvdG8="}}]}]"#,
    urls: &["https://a.example/report.pdf", "https://a.example/photo.png"],
}))]
#[case::leaves_non_remote_and_unsupported_parts_untouched(Scenario::Inline(InlineCase {
    filter: INLINE_EVERY_REMOTE_URL,
    concurrent: false,
    messages: r#"[{"role":"system","content":"plain text"},{"role":"user","content":[{"type":"image_url","image_url":{"url":"/etc/passwd"}},{"type":"image_url","image_url":"file:///etc/passwd"},{"type":"image_url","image_url":"data:image/png;base64,AAAA"},{"type":"file","file":{"file_id":"/tmp/secret","filename":"secret"}},{"type":"document","source":{"type":"base64","media_type":"application/pdf","data":"cGRm"}},{"type":"text","text":"keep"}]},{"role":"assistant","content":[]}]"#,
    data_urls: &[],
    expected: r#"[{"role":"system","content":"plain text"},{"role":"user","content":[{"type":"image_url","image_url":{"url":"/etc/passwd"}},{"type":"image_url","image_url":"file:///etc/passwd"},{"type":"image_url","image_url":"data:image/png;base64,AAAA"},{"type":"file","file":{"file_id":"/tmp/secret","filename":"secret"}},{"type":"document","source":{"type":"base64","media_type":"application/pdf","data":"cGRm"}},{"type":"text","text":"keep"}]},{"role":"assistant","content":[]}]"#,
    urls: &[],
}))]
#[case::image_filter_selects_only_image_url_parts(Scenario::Urls {
    filter: INLINE_REMOTE_IMAGE_URLS,
    messages: r#"[{"role":"user","content":[{"type":"image_url","image_url":{"url":"https://a.example/one.png"}},{"type":"document","source":{"type":"url","url":"https://a.example/doc.pdf"}},{"type":"file","file":{"file_id":"https://a.example/f.txt"}}]}]"#,
    expected: &["https://a.example/one.png"],
})]
#[case::deduplicates_urls_in_first_seen_order(Scenario::Urls {
    filter: INLINE_EVERY_REMOTE_URL,
    messages: r#"[{"role":"user","content":[{"type":"image_url","image_url":{"url":"https://a.example/b.png"}},{"type":"image_url","image_url":{"url":"https://a.example/a.png"}}]},{"role":"user","content":[{"type":"image_url","image_url":{"url":"https://a.example/b.png"}},{"type":"document","source":{"type":"url","url":"https://a.example/c.pdf"}}]},{"role":"user","content":[{"type":"image_url","image_url":{"url":"https://a.example/a.png"}}]}]"#,
    expected: &["https://a.example/b.png", "https://a.example/a.png", "https://a.example/c.pdf"],
})]
#[case::sequential_mode_converts_in_first_seen_order(Scenario::Inline(InlineCase {
    filter: INLINE_EVERY_REMOTE_URL,
    concurrent: false,
    messages: r#"[{"role":"user","content":[{"type":"image_url","image_url":{"url":"https://a.example/b.png"}},{"type":"image_url","image_url":{"url":"https://a.example/a.png"}},{"type":"image_url","image_url":{"url":"https://a.example/b.png"}}]}]"#,
    data_urls: &[("https://a.example/b.png", "data:image/png;base64,Yg=="), ("https://a.example/a.png", "data:image/png;base64,YQ==")],
    expected: r#"[{"role":"user","content":[{"type":"image_url","image_url":{"url":"data:image/png;base64,Yg=="}},{"type":"image_url","image_url":{"url":"data:image/png;base64,YQ=="}},{"type":"image_url","image_url":{"url":"data:image/png;base64,Yg=="}}]}]"#,
    urls: &["https://a.example/b.png", "https://a.example/a.png"],
}))]
#[case::sequential_failure_stops_at_the_first_url(Scenario::Failure {
    concurrent: false,
    messages: r#"[{"role":"user","content":[{"type":"image_url","image_url":{"url":"https://a.example/first.png"}},{"type":"image_url","image_url":{"url":"https://a.example/second.png"}}]}]"#,
    data_urls: &[("https://a.example/first.png", "data:image/png;base64,b25l")],
    missing: "https://a.example/second.png",
    converted: &["https://a.example/first.png", "https://a.example/second.png"],
})]
#[case::concurrent_failure_comes_from_the_batch(Scenario::Failure {
    concurrent: true,
    messages: r#"[{"role":"user","content":[{"type":"image_url","image_url":{"url":"https://a.example/first.png"}},{"type":"image_url","image_url":{"url":"https://a.example/second.png"}}]}]"#,
    data_urls: &[("https://a.example/first.png", "data:image/png;base64,b25l")],
    missing: "https://a.example/second.png",
    converted: &[],
})]
#[case::decode_requires_a_data_prefix(Scenario::Decode {
    url: "https://a.example/image.png",
    limit: 64,
    failure: DecodeFailure::MissingPrefix,
})]
#[case::decode_requires_the_base64_marker(Scenario::Decode {
    url: "data:image/png,hello",
    limit: 64,
    failure: DecodeFailure::MissingMarker,
})]
#[case::decode_rejects_oversized_payloads_before_decoding(Scenario::Decode {
    url: "data:image/png;base64,AAAAAA==",
    limit: 3,
    failure: DecodeFailure::TooLarge,
})]
#[case::decode_accepts_a_payload_at_the_limit(Scenario::DecodeOk {
    url: "data:image/png;base64,AAAAAA==",
    limit: 4,
    expected: &[0, 0, 0, 0],
})]
#[case::decode_counts_at_most_two_padding_characters(Scenario::Decode {
    url: "data:image/png;base64,AAAA========",
    limit: 3,
    failure: DecodeFailure::TooLarge,
})]
#[case::decode_rejects_invalid_base64(Scenario::Decode {
    url: "data:image/png;base64,!!!!",
    limit: 64,
    failure: DecodeFailure::InvalidBase64,
})]
#[case::decode_rejects_non_canonical_base64(Scenario::Decode {
    url: "data:image/png;base64,AB==",
    limit: 64,
    failure: DecodeFailure::InvalidBase64,
})]
#[case::decode_returns_the_payload_bytes(Scenario::DecodeOk {
    url: "data:text/plain;base64,aGVsbG8=",
    limit: 5,
    expected: b"hello",
})]
#[tokio::test]
async fn inline_media_tests_behavior(#[case] scenario: Scenario) {
    run(scenario).await;
}

#[test]
fn media_base64_string_component() {
    assert_eq!(get_base64_str("data:image/png;base64,YWJj"), "YWJj");
    assert_eq!(get_base64_str("YWJj"), "YWJj");
    assert_eq!(get_base64_str("prefix,"), "");
    assert_eq!(get_base64_str("a,b,c"), "b");
    assert_eq!(get_base64_str(""), "");
}

#[test]
fn media_base64_classification() {
    assert!(is_base64_encoded("data:image/png;base64,YWJj"));
    assert!(is_base64_encoded("data:image/png;base64,"));
    assert!(!is_base64_encoded("YWJj"));
    assert!(!is_base64_encoded("data:image/png;base64,!!!bad"));
    assert!(!is_base64_encoded("data:image/png;base64,YR=="));
    assert!(!is_base64_encoded("data:image/png;base64,AAAA===="));
    assert!(!is_base64_encoded("data:image/png;base64,AAAAAA="));
    assert!(is_base64_encoded("data:text/plain;base64,YQ=="));
    assert!(!is_canonical_base64("AAAA", 2));
    assert!(is_canonical_base64("AAAA", 3));
}

#[rstest]
fn replays_the_media_data_corpus(#[files("fuzz/corpus/media_data/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    let Ok(text) = str::from_utf8(&bytes) else {
        return;
    };
    drop(decode_data_url(text, MAX_MEDIA_BYTES));
    black_box(is_base64_encoded(text));
}
