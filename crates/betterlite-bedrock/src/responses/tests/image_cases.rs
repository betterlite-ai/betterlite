use std::collections::BTreeMap;
use std::sync::PoisonError;

use betterlite_prompts::media::error::MediaError;
use betterlite_prompts::media::limits::MAX_REMOTE_URLS;
use betterlite_prompts::media_fetch::error::ImageFetchError;
use serde_json::Value;

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
        Err(ResponsesError::MissingInlined { ref url }) if url == "http://a.example/1.png"
    ));
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
    let singles = sync
        .fetcher
        .singles
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    assert_eq!(singles, strings(&FETCHED_ORDER));
    assert!(
        sync.fetcher
            .batches
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_empty()
    );
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
    let batches = parallel
        .fetcher
        .batches
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    assert_eq!(batches, vec![strings(&FETCHED_ORDER)]);
    assert!(
        parallel
            .fetcher
            .singles
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_empty()
    );
    assert_eq!(sync_body, async_body);
    let sent = serde_json::to_value(&sync_body).expect("the body encodes");
    assert_eq!(
        sent.pointer("/input/0/content/0/image_url"),
        Some(&Value::from("data:image/png;base64,QQ=="))
    );
    assert_eq!(
        sent.pointer("/input/0/content/2/image_url"),
        Some(&Value::from("data:image/png;base64,QQ=="))
    );
    assert_eq!(
        sent.pointer("/input/1/output/image_url"),
        Some(&Value::from("data:image/png;base64,Qw=="))
    );
    let refusing = BedrockResponsesConfig {
        fetcher: MapFetcher::with(&[]),
    };
    let refused = block_on(transform_responses_request(
        &refusing.fetcher,
        MODEL,
        &source,
        &object("{}"),
        None,
    ));
    assert!(matches!(
        refused,
        Err(ResponsesError::Fetch(ImageFetchError::Rejected { .. }))
    ));
    let refused_async = block_on(async_transform_responses(
        &refusing.fetcher,
        MODEL,
        &source,
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
    let config = BedrockResponsesConfig {
        fetcher: MapFetcher::with(&DATA_URLS),
    };
    let sync = block_on(transform_responses_request(
        &config.fetcher,
        MODEL,
        &source,
        &object("{}"),
        None,
    ))
    .expect("sync fetch");
    let parallel = block_on(async_transform_responses(
        &config.fetcher,
        MODEL,
        &source,
        &object("{}"),
        None,
    ))
    .expect("async fetch");
    let sent = serde_json::to_value(&sync).expect("the body encodes");
    assert_eq!(
        sent.pointer("/input/0/content/0/image_url"),
        Some(&Value::from("data:image/png;base64,QQ=="))
    );
    assert_eq!(sync, parallel);
    let refusing = BedrockResponsesConfig {
        fetcher: MapFetcher::with(&[]),
    };
    let refused = block_on(transform_responses_request(
        &refusing.fetcher,
        MODEL,
        &source,
        &object("{}"),
        None,
    ));
    assert!(matches!(refused, Err(ResponsesError::Fetch(_))));
}

pub(super) fn remote_url_limit() {
    let blocks = |count: usize| {
        let images: Vec<String> = (0..count)
            .map(|index| {
                format!(r#"{{"type": "input_image", "image_url": "http://m.example/{index}.png"}}"#)
            })
            .collect();
        input(&format!(
            r#"[{{"type": "message", "role": "user", "content": [{}]}}]"#,
            images.join(",")
        ))
    };
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
    let config = BedrockResponsesConfig {
        fetcher: MapFetcher::with(&pairs),
    };
    let within = block_on(transform_responses_request(
        &config.fetcher,
        MODEL,
        &blocks(MAX_REMOTE_URLS),
        &object("{}"),
        None,
    ));
    assert!(within.is_ok());
    let over = block_on(async_transform_responses(
        &config.fetcher,
        MODEL,
        &blocks(MAX_REMOTE_URLS + 1),
        &object("{}"),
        None,
    ));
    assert!(
        matches!(over, Err(ResponsesError::Media(MediaError::TooManyRemoteUrls { limit })) if limit == MAX_REMOTE_URLS)
    );
    let over_sync = block_on(transform_responses_request(
        &config.fetcher,
        MODEL,
        &blocks(MAX_REMOTE_URLS + 1),
        &object("{}"),
        None,
    ));
    assert!(matches!(over_sync, Err(ResponsesError::Media(_))));
    let calls = config
        .fetcher
        .singles
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .len();
    assert_eq!(calls, MAX_REMOTE_URLS);
    assert!(
        config
            .fetcher
            .batches
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_empty()
    );
}
