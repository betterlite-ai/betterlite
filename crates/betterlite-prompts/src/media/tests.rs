#![cfg(test)]

use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::hint::black_box;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::{fs, str};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::tls::Verification;
use betterlite_config::types::ProxyTrust;
use betterlite_context::core_settings::CoreSettings;
use betterlite_http::proxy::{ProxiedClient, base_builder};
use betterlite_http::user_url::fetch::UserUrlFetcher;
use betterlite_http::user_url::validate::{SystemResolver, UrlValidator};
use rstest::rstest;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;
use tokio::task::JoinSet;

use super::base64_string::{get_base64_str, is_base64_encoded};
use super::data_url::{MAX_MEDIA_BYTES, decode_data_url, is_canonical_base64};
use super::error::MediaError;
use super::inline::{
    RemoteMediaFetcher, async_inline_remote_media, inline_remote_media, remote_urls_to_inline,
};
use super::limits::{MAX_MESSAGE_PARTS, MAX_MESSAGES, MAX_REMOTE_URLS};
use super::parts::{INLINE_EVERY_REMOTE_URL, INLINE_REMOTE_IMAGE_URLS, RemoteMedia};
use crate::arguments::json::{Json, Object};
use crate::media_fetch::error::ImageFetchError;
use crate::media_fetch::fetcher::Fetcher;
use crate::media_fetch::limits::transport_limits;
use crate::message_content::{Content, ContentPart, Message};

type Filter = fn(&RemoteMedia) -> bool;

struct InlineCase {
    filter: Filter,
    concurrent: bool,
    messages: &'static str,
    data_urls: &'static [(&'static str, &'static str)],
    expected: &'static str,
    urls: &'static [&'static str],
}

#[derive(Clone, Copy)]
enum CapExpect {
    Collected(usize),
    TooManyMessages,
    TooManyMessageParts,
    TooManyRemoteUrls,
}

enum Scenario {
    Inline(InlineCase),
    Urls {
        filter: Filter,
        messages: &'static str,
        expected: &'static [&'static str],
    },
    Fields {
        messages: &'static str,
        accepted: &'static [&'static str],
        observed: &'static [(&'static str, &'static str)],
    },
    Caps {
        messages: usize,
        parts: usize,
        distinct_urls: usize,
        expected: CapExpect,
    },
    Failure {
        concurrent: bool,
        messages: &'static str,
        data_urls: &'static [(&'static str, &'static str)],
        missing: &'static str,
        converted: &'static [&'static str],
    },
    RealRejected {
        url: &'static str,
    },
    RealFetched,
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

fn real_fetcher(allowed: &[&str]) -> Fetcher<SystemResolver> {
    let policy = Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap());
    let core = CoreSettings {
        request_timeout_seconds: 6000.0,
        request_timeout_explicit: false,
        litellm_request_timeout: None,
        model_timeout_overrides: Vec::new(),
    };
    let client = ProxiedClient::build(base_builder(), &policy, transport_limits(&core)).unwrap();
    let allowed_hosts: Vec<String> = allowed.iter().map(|host| (*host).to_owned()).collect();
    let validator = UrlValidator::new(SystemResolver, &allowed_hosts, &Verification::Enabled);
    Fetcher::new(
        client,
        UserUrlFetcher {
            validator,
            validation: true,
        },
        50.0_f64,
        Arc::new(|| 0.0_f64),
    )
    .unwrap()
}

async fn serve_once(body: &[u8]) -> (SocketAddr, JoinSet<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let head = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let payload = body.to_vec();
    let mut tasks = JoinSet::new();
    tasks.spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        let mut chunk = [0_u8; 1024];
        while !request.windows(4).any(|window| window == b"\r\n\r\n") {
            let count = stream.read(&mut chunk).await.unwrap();
            assert!(count > 0);
            request.extend_from_slice(chunk.get(..count).unwrap());
        }
        stream.write_all(head.as_bytes()).await.unwrap();
        stream.write_all(&payload).await.unwrap();
        stream.flush().await.unwrap();
    });
    (address, tasks)
}

fn capped_urls(distinct: usize) -> Vec<String> {
    (0..distinct)
        .map(|index| format!("https://a.example/{index}.png"))
        .collect()
}

fn image_part(url: &str) -> ContentPart {
    let image_url = Object::from_iter([("url".to_owned(), Json::Text(url.to_owned()))]);
    ContentPart {
        kind: "image_url".to_owned(),
        fields: Object::from_iter([("image_url".to_owned(), Json::Object(image_url))]),
    }
}

fn user_message(parts: Vec<ContentPart>) -> Message {
    Message {
        role: "user".to_owned(),
        content: Some(Content::Parts(parts)),
        name: None,
        provider_specific_fields: Object::new(),
    }
}

fn capped_messages(
    message_count: usize,
    parts_per_message: usize,
    distinct_urls: usize,
) -> Vec<Message> {
    let mut urls = capped_urls(distinct_urls).into_iter().cycle();
    let mut messages = Vec::with_capacity(message_count);
    for _message in 0..message_count {
        let mut content = Vec::with_capacity(parts_per_message);
        for _part in 0..parts_per_message {
            let url = urls.next().unwrap();
            content.push(image_part(&url));
        }
        messages.push(user_message(content));
    }
    messages
}

async fn run(scenario: Scenario) {
    match scenario {
        Scenario::Inline(case) => run_inline(case).await,
        Scenario::Urls {
            filter,
            messages,
            expected,
        } => run_urls(filter, messages, expected),
        Scenario::Fields {
            messages,
            accepted,
            observed,
        } => run_fields(messages, accepted, observed),
        Scenario::Caps {
            messages,
            parts,
            distinct_urls,
            expected,
        } => run_caps(messages, parts, distinct_urls, expected),
        Scenario::Failure {
            concurrent,
            messages,
            data_urls,
            missing,
            converted,
        } => run_failure(concurrent, messages, data_urls, missing, converted).await,
        Scenario::RealRejected { url } => run_real_rejected(url).await,
        Scenario::RealFetched => run_real_fetched().await,
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
        remote_urls_to_inline(&message_list, &case.filter).unwrap(),
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
        remote_urls_to_inline(&message_list, &filter).unwrap(),
        owned(expected)
    );
}

fn run_fields(messages: &str, accepted: &[&str], observed: &[(&str, &str)]) {
    let message_list = messages_of(messages);
    let seen = Mutex::new(Vec::new());
    let predicate = |media: &RemoteMedia| {
        seen.lock()
            .unwrap()
            .push((media.url.clone(), media.fields.clone()));
        !media.fields.contains_key("format")
    };
    let urls = remote_urls_to_inline(&message_list, &predicate).unwrap();
    assert_eq!(urls, owned(accepted));
    let actual: Vec<(String, Json)> = {
        let recorded = seen.lock().unwrap();
        recorded
            .iter()
            .map(|(url, fields)| (url.clone(), Json::Object(fields.clone())))
            .collect()
    };
    let expected: Vec<(String, Json)> = observed
        .iter()
        .map(|(url, fields)| ((*url).to_owned(), parsed(fields)))
        .collect();
    assert_eq!(actual, expected);
}

fn run_caps(message_count: usize, parts: usize, distinct_urls: usize, expected: CapExpect) {
    let messages = capped_messages(message_count, parts, distinct_urls);
    let outcome = remote_urls_to_inline(&messages, &INLINE_EVERY_REMOTE_URL);
    match expected {
        CapExpect::Collected(count) => assert_eq!(outcome.unwrap(), capped_urls(count)),
        CapExpect::TooManyMessages => {
            assert!(matches!(outcome, Err(MediaError::TooManyMessages { .. })));
        }
        CapExpect::TooManyMessageParts => {
            assert!(matches!(
                outcome,
                Err(MediaError::TooManyMessageParts { .. })
            ));
        }
        CapExpect::TooManyRemoteUrls => {
            assert!(matches!(outcome, Err(MediaError::TooManyRemoteUrls { .. })));
        }
    }
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

async fn run_real_rejected(url: &str) {
    let messages = vec![user_message(vec![image_part(url)])];
    let source = real_fetcher(&[]);
    let outcome = async_inline_remote_media(&messages, &INLINE_EVERY_REMOTE_URL, &source).await;
    let error = outcome.unwrap_err();
    assert_eq!(
        error.to_string(),
        ImageFetchError::Rejected {
            url: url.to_owned()
        }
        .to_string()
    );
    assert!(matches!(
        error,
        MediaError::Fetch(ImageFetchError::Rejected { .. })
    ));
}

async fn run_real_fetched() {
    const IMAGE: &[u8] = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR";
    let (address, mut tasks) = serve_once(IMAGE).await;
    let url = format!("http://{address}/image.png");
    let messages = vec![user_message(vec![image_part(&url)])];
    let source = real_fetcher(&["127.0.0.1"]);
    let inlined = async_inline_remote_media(&messages, &INLINE_EVERY_REMOTE_URL, &source)
        .await
        .unwrap();
    let data_url = format!("data:image/png;base64,{}", BASE64.encode(IMAGE));
    assert_eq!(inlined, vec![user_message(vec![image_part(&data_url)])]);
    while tasks.join_next().await.is_some() {}
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
#[case::infers_pdf_format_for_an_uppercase_url(Scenario::Inline(InlineCase {
    filter: INLINE_EVERY_REMOTE_URL,
    concurrent: false,
    messages: r#"[{"role":"user","content":[{"type":"file","file":{"file_id":"https://a.example/REPORT.PDF"}},{"type":"document","source":{"type":"url","url":"https://a.example/SCAN.PDF"}}]}]"#,
    data_urls: &[("https://a.example/REPORT.PDF", "data:image/png;base64,cGRm"), ("https://a.example/SCAN.PDF", "data:image/png;base64,c2Nhbg==")],
    expected: r#"[{"role":"user","content":[{"type":"file","file":{"format":"application/pdf","file_data":"data:image/png;base64,cGRm"}},{"type":"document","source":{"type":"base64","media_type":"application/pdf","data":"c2Nhbg=="}}]}]"#,
    urls: &["https://a.example/REPORT.PDF", "https://a.example/SCAN.PDF"],
}))]
#[case::inlines_document_and_image_sources(Scenario::Inline(InlineCase {
    filter: INLINE_EVERY_REMOTE_URL,
    concurrent: true,
    messages: r#"[{"role":"user","content":[{"type":"document","title":"Report","source":{"type":"url","url":"https://a.example/report.pdf","media_type":"application/pdf"}},{"type":"image","source":{"type":"url","url":"https://a.example/photo.png"}}]}]"#,
    data_urls: &[("https://a.example/report.pdf", "data:foo/bar;base64,cGRm"), ("https://a.example/photo.png", "data:image/png;base64,cGhvdG8=")],
    expected: r#"[{"role":"user","content":[{"type":"document","title":"Report","source":{"type":"base64","media_type":"application/pdf","data":"cGRm"}},{"type":"image","source":{"type":"base64","media_type":"image/png","data":"cGhvdG8="}}]}]"#,
    urls: &["https://a.example/report.pdf", "https://a.example/photo.png"],
}))]
#[case::image_filter_leaves_a_shared_document_url_alone(Scenario::Inline(InlineCase {
    filter: INLINE_REMOTE_IMAGE_URLS,
    concurrent: false,
    messages: r#"[{"role":"user","content":[{"type":"image_url","image_url":{"url":"https://a.example/shared.png"}},{"type":"document","source":{"type":"url","url":"https://a.example/shared.png"}}]}]"#,
    data_urls: &[("https://a.example/shared.png", "data:image/png;base64,c2hhcmVk")],
    expected: r#"[{"role":"user","content":[{"type":"image_url","image_url":{"url":"data:image/png;base64,c2hhcmVk"}},{"type":"document","source":{"type":"url","url":"https://a.example/shared.png"}}]}]"#,
    urls: &["https://a.example/shared.png"],
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
    messages: r#"[{"role":"user","content":[{"type":"image_url","image_url":{"url":"https://a.example/one.png"}},{"type":"document","source":{"type":"url","url":"https://a.example/doc.pdf"}},{"type":"file","file":{"file_id":"https://a.example/f.txt"}},{"type":"image","source":{"type":"url","url":"https://a.example/two.png"}}]}]"#,
    expected: &["https://a.example/one.png"],
})]
#[case::deduplicates_urls_in_first_seen_order(Scenario::Urls {
    filter: INLINE_EVERY_REMOTE_URL,
    messages: r#"[{"role":"user","content":[{"type":"image_url","image_url":{"url":"https://a.example/b.png"}},{"type":"image_url","image_url":{"url":"https://a.example/a.png"}}]},{"role":"user","content":[{"type":"image_url","image_url":{"url":"https://a.example/b.png"}},{"type":"document","source":{"type":"url","url":"https://a.example/c.pdf"}}]},{"role":"user","content":[{"type":"image_url","image_url":{"url":"https://a.example/a.png"}}]}]"#,
    expected: &["https://a.example/b.png", "https://a.example/a.png", "https://a.example/c.pdf"],
})]
#[case::passes_each_shapes_fields_to_the_predicate(Scenario::Fields {
    messages: r#"[{"role":"user","content":[{"type":"image_url","image_url":{"url":"https://a.example/with-format.png","format":"image/png"}},{"type":"image_url","image_url":"https://a.example/bare.png"},{"type":"file","file":{"file_id":"https://a.example/report.pdf","format":"application/pdf"}},{"type":"document","source":{"type":"url","url":"https://a.example/doc.pdf","media_type":"application/pdf"}}]}]"#,
    accepted: &["https://a.example/bare.png", "https://a.example/doc.pdf"],
    observed: &[
        ("https://a.example/with-format.png", r#"{"url":"https://a.example/with-format.png","format":"image/png"}"#),
        ("https://a.example/bare.png", "{}"),
        ("https://a.example/report.pdf", r#"{"file_id":"https://a.example/report.pdf","format":"application/pdf"}"#),
        ("https://a.example/doc.pdf", r#"{"type":"url","url":"https://a.example/doc.pdf","media_type":"application/pdf"}"#),
    ],
})]
#[case::cap_accepts_the_message_limit(Scenario::Caps {
    messages: MAX_MESSAGES,
    parts: 1,
    distinct_urls: 1,
    expected: CapExpect::Collected(1),
})]
#[case::cap_rejects_more_than_the_message_limit(Scenario::Caps {
    messages: MAX_MESSAGES.saturating_add(1),
    parts: 1,
    distinct_urls: 1,
    expected: CapExpect::TooManyMessages,
})]
#[case::cap_accepts_the_part_limit(Scenario::Caps {
    messages: 1,
    parts: MAX_MESSAGE_PARTS,
    distinct_urls: MAX_REMOTE_URLS,
    expected: CapExpect::Collected(MAX_REMOTE_URLS),
})]
#[case::cap_rejects_more_than_the_part_limit(Scenario::Caps {
    messages: 1,
    parts: MAX_MESSAGE_PARTS.saturating_add(1),
    distinct_urls: MAX_REMOTE_URLS,
    expected: CapExpect::TooManyMessageParts,
})]
#[case::cap_accepts_the_url_limit(Scenario::Caps {
    messages: 2,
    parts: 128,
    distinct_urls: MAX_REMOTE_URLS,
    expected: CapExpect::Collected(MAX_REMOTE_URLS),
})]
#[case::cap_rejects_more_than_the_url_limit(Scenario::Caps {
    messages: 2,
    parts: 129,
    distinct_urls: MAX_REMOTE_URLS.saturating_add(1),
    expected: CapExpect::TooManyRemoteUrls,
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
#[case::real_fetcher_rejects_a_blocked_url(Scenario::RealRejected {
    url: "http://127.0.0.1:9/image.png",
})]
#[case::real_fetcher_inlines_a_fetched_image(Scenario::RealFetched)]
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
    assert!(is_base64_encoded("data:text/plain;base64,YWJj,ignored"));
    assert!(is_base64_encoded("data:text/plain,YQ=="));
    assert!(!is_base64_encoded("data:image/png;base64"));
    assert!(!is_base64_encoded("DATA:image/png;base64,YWJj"));
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
