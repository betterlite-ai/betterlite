use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};

use betterlite_openai::response_input::ResponsesInput;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::media::error::MediaError;
use betterlite_prompts::media::limits::MAX_REMOTE_URLS;
use betterlite_prompts::media_fetch::error::ImageFetchError;

use crate::responses::config::BedrockResponsesConfig;
use crate::responses::error::ResponsesError;
use crate::responses::images::{collect_remote_image_urls, inline_remote_image_urls};
use crate::responses::request::{async_transform_responses, transform_responses_request};

use super::support::{MODEL, MapFetcher, block_on, input, object};

const TRAVERSAL: &str = r#"[
    {"type": "message", "role": "user", "content": [
        {"type": "input_text", "text": "hi"},
        {"type": "input_image", "image_url": "http://a.example/1.png"},
        {"type": "input_image", "image_url": "https://b.example/2.png"},
        {"type": "input_image", "image_url": "http://a.example/1.png"},
        {"type": "input_image", "image_url": {"url": "http://nested.example/x.png"}},
        {"type": "input_image", "image_url": 7},
        {"type": "input_image", "image_url": "HTTP://upper.example/x.png"},
        {"type": "input_image", "image_url": "s3://bucket/key.png"},
        {"type": "input_image", "image_url": "data:image/png;base64,AAAA"},
        {"type": "input_file", "image_url": "http://file.example/x.png"},
        {"type": "input_image", "file_url": "http://file.example/y.png"},
        {"type": "input_text", "content": [{"type": "input_image", "image_url": "http://deep.example/x.png"}]}
    ]},
    {"type": "function_call_output", "call_id": "c1", "output": [{"type": "input_image", "image_url": "https://out.example/3.png"}]},
    {"type": "computer_call_output", "call_id": "c2", "output": {"type": "computer_screenshot", "image_url": "http://shot.example/4.png"}},
    {"type": "message", "role": "user", "content": {"type": "input_image", "image_url": "http://single.example/5.png"}, "output": [{"type": "input_image", "image_url": "http://both.example/6.png"}]},
    {"type": "message", "role": "user", "content": "http://text.example/7.png"},
    "http://loose.example/8.png",
    7
]"#;

const REPLACEMENT: &str = r#"[
    {"type": "message", "role": "user", "content": [
        {"image_url": "http://a.example/1.png", "type": "input_image", "detail": "auto"},
        {"type": "input_file", "image_url": "http://a.example/1.png"},
        {"type": "input_image", "image_url": "s3://bucket/key.png"},
        {"type": "input_image", "image_url": "HTTP://a.example/1.png"},
        "loose"
    ], "id": "m1"},
    {"type": "computer_call_output", "call_id": "c", "output": {"type": "computer_screenshot", "image_url": "https://b.example/2.png"}},
    {"type": "message", "role": "user", "content": "http://a.example/1.png"},
    7
]"#;

const REPLACED: &str = r#"[
    {"type": "message", "role": "user", "content": [
        {"image_url": "data:image/png;base64,QQ==", "type": "input_image", "detail": "auto"},
        {"type": "input_file", "image_url": "http://a.example/1.png"},
        {"type": "input_image", "image_url": "s3://bucket/key.png"},
        {"type": "input_image", "image_url": "HTTP://a.example/1.png"},
        "loose"
    ], "id": "m1"},
    {"type": "computer_call_output", "call_id": "c", "output": {"type": "computer_screenshot", "image_url": "data:image/png;base64,Qg=="}},
    {"type": "message", "role": "user", "content": "http://a.example/1.png"},
    7
]"#;

const FETCHED: &str = r#"[
    {"type": "message", "role": "user", "content": [
        {"type": "input_image", "image_url": "http://a.example/1.png"},
        {"type": "input_image", "image_url": "https://b.example/2.png"},
        {"type": "input_image", "image_url": "http://a.example/1.png"}
    ]},
    {"type": "computer_call_output", "call_id": "c", "output": {"type": "computer_screenshot", "image_url": "http://shot.example/3.png"}}
]"#;

const DATA_URLS: [(&str, &str); 3] = [
    ("http://a.example/1.png", "data:image/png;base64,QQ=="),
    ("https://b.example/2.png", "data:image/png;base64,Qg=="),
    ("http://shot.example/3.png", "data:image/png;base64,Qw=="),
];

const FETCHED_ORDER: [&str; 3] = [
    "http://a.example/1.png",
    "https://b.example/2.png",
    "http://shot.example/3.png",
];

fn strings(urls: &[&str]) -> Vec<String> {
    urls.iter().map(|url| (*url).to_owned()).collect()
}

fn mapping(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(url, data)| ((*url).to_owned(), (*data).to_owned()))
        .collect()
}

pub(super) fn inline_image_traversal() {
    assert_eq!(
        collect_remote_image_urls(&input(TRAVERSAL)),
        strings(&[
            "http://a.example/1.png",
            "https://b.example/2.png",
            "https://out.example/3.png",
            "http://shot.example/4.png",
            "http://single.example/5.png",
            "http://both.example/6.png",
        ])
    );
    assert!(collect_remote_image_urls(&input(r#""http://text.example/x.png""#)).is_empty());
    assert!(collect_remote_image_urls(&input("[]")).is_empty());
}

pub(super) fn inline_image_replacement() {
    let source = input(REPLACEMENT);
    let replaced = inline_remote_image_urls(
        &source,
        &mapping(&[
            ("http://a.example/1.png", "data:image/png;base64,QQ=="),
            ("https://b.example/2.png", "data:image/png;base64,Qg=="),
        ]),
    )
    .expect("every URL is mapped");
    assert_eq!(
        serde_json::to_string(&replaced).expect("the input encodes"),
        serde_json::to_string(&input(REPLACED)).expect("the input encodes")
    );
    let text = input(r#""http://a.example/1.png""#);
    let text_map = mapping(&[("http://a.example/1.png", "data:x")]);
    assert_eq!(
        inline_remote_image_urls(&text, &text_map).expect("text passes"),
        text
    );
    let unchanged = inline_remote_image_urls(&source, &BTreeMap::new()).expect("no mapping passes");
    assert_eq!(unchanged, source);
    let missing = inline_remote_image_urls(
        &source,
        &mapping(&[("http://other.example/x.png", "data:x")]),
    );
    assert!(matches!(
        missing,
        Err(ResponsesError::MissingInlined { url }) if url == "http://a.example/1.png"
    ));
}

fn fetch_log(calls: &Mutex<Vec<String>>) -> Vec<String> {
    calls.lock().unwrap_or_else(PoisonError::into_inner).clone()
}

fn batch_log(calls: &Mutex<Vec<Vec<String>>>) -> Vec<Vec<String>> {
    calls.lock().unwrap_or_else(PoisonError::into_inner).clone()
}

fn image_at(body: &Object, pointer: &str) -> Option<String> {
    let mut current = Json::Object(body.clone());
    for segment in pointer.split('/').skip(1) {
        current = match &current {
            Json::Object(fields) => fields.get(segment)?.clone(),
            Json::List(items) => items.get(segment.parse::<usize>().ok()?)?.clone(),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) => return None,
        };
    }
    current.as_str().map(str::to_owned)
}

pub(super) fn remote_url_inlined() {
    let source = input(FETCHED);
    let sync = BedrockResponsesConfig {
        fetcher: MapFetcher::with(&DATA_URLS),
    };
    let sync_body = block_on(transform_responses_request(
        &sync.fetcher,
        MODEL,
        &source,
        &object("{}"),
        None,
    ))
    .expect("the sync transform fetches");
    assert_eq!(fetch_log(&sync.fetcher.singles), strings(&FETCHED_ORDER));
    assert!(batch_log(&sync.fetcher.batches).is_empty());
    let parallel = BedrockResponsesConfig {
        fetcher: MapFetcher::with(&DATA_URLS),
    };
    let async_body = block_on(async_transform_responses(
        &parallel.fetcher,
        MODEL,
        &source,
        &object("{}"),
        None,
    ))
    .expect("the async transform fetches");
    assert_eq!(
        batch_log(&parallel.fetcher.batches),
        vec![strings(&FETCHED_ORDER)]
    );
    assert!(fetch_log(&parallel.fetcher.singles).is_empty());
    assert_eq!(sync_body, async_body);
    assert_inlined(&sync_body);
    refusals(&source);
}

fn assert_inlined(body: &Object) {
    for (pointer, expected) in [
        ("/input/0/content/0/image_url", "data:image/png;base64,QQ=="),
        ("/input/0/content/1/image_url", "data:image/png;base64,Qg=="),
        ("/input/0/content/2/image_url", "data:image/png;base64,QQ=="),
        ("/input/1/output/image_url", "data:image/png;base64,Qw=="),
    ] {
        assert_eq!(image_at(body, pointer), Some(expected.to_owned()));
    }
}

fn refusals(source: &ResponsesInput) {
    let refusing = MapFetcher::with(&[]);
    let refused = block_on(transform_responses_request(
        &refusing,
        MODEL,
        source,
        &object("{}"),
        None,
    ));
    assert!(matches!(
        refused,
        Err(ResponsesError::Fetch(ImageFetchError::Rejected { .. }))
    ));
    let refused_async = block_on(async_transform_responses(
        &refusing,
        MODEL,
        source,
        &object("{}"),
        None,
    ));
    assert!(matches!(
        refused_async,
        Err(ResponsesError::Fetch(ImageFetchError::Rejected { .. }))
    ));
}

pub(super) fn responses_media_fetch() {
    let source = input(
        r#"[{"type": "message", "role": "user", "content": [{"type": "input_image", "image_url": "http://a.example/1.png"}]}]"#,
    );
    let fetcher = MapFetcher::with(&DATA_URLS);
    let sync = block_on(transform_responses_request(
        &fetcher,
        MODEL,
        &source,
        &object("{}"),
        None,
    ))
    .expect("sync fetch");
    let parallel = block_on(async_transform_responses(
        &fetcher,
        MODEL,
        &source,
        &object("{}"),
        None,
    ))
    .expect("async fetch");
    assert_eq!(
        image_at(&sync, "/input/0/content/0/image_url"),
        Some("data:image/png;base64,QQ==".to_owned())
    );
    assert_eq!(sync, parallel);
    let refused = block_on(transform_responses_request(
        &MapFetcher::with(&[]),
        MODEL,
        &source,
        &object("{}"),
        None,
    ));
    assert!(matches!(refused, Err(ResponsesError::Fetch(_))));
}

fn many_images(count: usize) -> ResponsesInput {
    let images: Vec<String> = (0..count)
        .map(|index| {
            format!(r#"{{"type": "input_image", "image_url": "http://m.example/{index}.png"}}"#)
        })
        .collect();
    input(&format!(
        r#"[{{"type": "message", "role": "user", "content": [{}]}}]"#,
        images.join(",")
    ))
}

fn limit_fetcher() -> MapFetcher {
    let all: Vec<(String, String)> = (0..=MAX_REMOTE_URLS)
        .map(|index| {
            (
                format!("http://m.example/{index}.png"),
                "data:image/png;base64,QQ==".to_owned(),
            )
        })
        .collect();
    let pairs: Vec<(&str, &str)> = all
        .iter()
        .map(|(url, data)| (url.as_str(), data.as_str()))
        .collect();
    MapFetcher::with(&pairs)
}

pub(super) fn remote_url_limit() {
    let fetcher = limit_fetcher();
    let within = block_on(transform_responses_request(
        &fetcher,
        MODEL,
        &many_images(MAX_REMOTE_URLS),
        &object("{}"),
        None,
    ));
    within.expect("the limit itself is allowed");
    let over = block_on(async_transform_responses(
        &fetcher,
        MODEL,
        &many_images(MAX_REMOTE_URLS + 1),
        &object("{}"),
        None,
    ));
    assert!(matches!(
        over,
        Err(ResponsesError::Media(MediaError::TooManyRemoteUrls { limit })) if limit == MAX_REMOTE_URLS
    ));
    let over_sync = block_on(transform_responses_request(
        &fetcher,
        MODEL,
        &many_images(MAX_REMOTE_URLS + 1),
        &object("{}"),
        None,
    ));
    assert!(matches!(over_sync, Err(ResponsesError::Media(_))));
    assert_eq!(fetch_log(&fetcher.singles).len(), MAX_REMOTE_URLS);
    assert!(batch_log(&fetcher.batches).is_empty());
}
