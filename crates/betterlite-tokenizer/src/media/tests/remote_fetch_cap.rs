use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};

use betterlite_prompts::media::limits::MAX_REMOTE_URLS;
use rstest::rstest;

use crate::media::error::MediaError;
use crate::media::settings::ImageCounting;

use super::support::{self, CaseOptions, MappingFetcher, plain};

const PNG: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAABAAAAAQA";
const TOO_MANY: &str = "the content asks for more than 256 remote image downloads";

const EXACT: CaseOptions = CaseOptions {
    counting: ImageCounting::Exact,
    fallback: None,
    image_default_count: 250,
};
const DEFAULT: CaseOptions = CaseOptions {
    counting: ImageCounting::DefaultCount,
    fallback: None,
    image_default_count: 250,
};

#[derive(Clone, Copy)]
enum Layout {
    Flat,
    ToolResult,
}

fn distinct_urls(count: usize) -> Vec<String> {
    (0..count)
        .map(|index| format!("https://example.com/image-{index}.png"))
        .collect()
}

fn mapping(urls: &[String]) -> BTreeMap<String, String> {
    urls.iter()
        .map(|url| (url.clone(), PNG.to_owned()))
        .collect()
}

fn image_item(url: &str, detail: &str) -> String {
    format!(r#"{{"type":"image_url","image_url":{{"url":"{url}","detail":"{detail}"}}}}"#)
}

fn image_content(urls: &[String], layout: Layout) -> String {
    let mut items: Vec<String> = urls.iter().map(|url| image_item(url, "high")).collect();
    match layout {
        Layout::Flat => plain(&items.join(",")),
        Layout::ToolResult => {
            let nested = items.pop().unwrap_or_default();
            items.push(format!(
                r#"{{"type":"tool_result","tool_use_id":"toolu_1","content":[{nested}]}}"#
            ));
            plain(&items.join(","))
        }
    }
}

#[tokio::test]
async fn counts_up_to_the_fetch_cap() {
    let urls = distinct_urls(MAX_REMOTE_URLS);
    let fetcher = MappingFetcher {
        urls: mapping(&urls),
        calls: AtomicUsize::new(0),
    };
    let fixture = support::words_fixture();
    let settings = support::settings(&EXACT);
    let counted = support::count(
        &fixture,
        &image_content(&urls, Layout::Flat),
        &settings,
        &fetcher,
    )
    .await
    .unwrap();
    // 256 images of 765 tokens each
    assert_eq!(counted.tokens, 195_840);
    assert_eq!(fetcher.calls.load(Ordering::SeqCst), MAX_REMOTE_URLS);
}

#[rstest]
#[case::top_level(Layout::Flat)]
#[case::nested_tool_result(Layout::ToolResult)]
#[tokio::test]
async fn refuses_past_the_fetch_cap(#[case] layout: Layout) {
    let urls = distinct_urls(MAX_REMOTE_URLS + 1);
    let fetcher = MappingFetcher {
        urls: mapping(&urls),
        calls: AtomicUsize::new(0),
    };
    let fixture = support::words_fixture();
    let settings = support::settings(&EXACT);
    let error = support::count(&fixture, &image_content(&urls, layout), &settings, &fetcher)
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), TOO_MANY);
    assert!(
        matches!(error, MediaError::TooManyRemoteFetches { limit } if limit == MAX_REMOTE_URLS)
    );
    assert_eq!(fetcher.calls.load(Ordering::SeqCst), MAX_REMOTE_URLS);
}

#[tokio::test]
async fn low_detail_never_counts() {
    let urls = distinct_urls(300);
    let fetcher = MappingFetcher {
        urls: mapping(&urls),
        calls: AtomicUsize::new(0),
    };
    let images: Vec<String> = urls.iter().map(|url| image_item(url, "low")).collect();
    let fixture = support::words_fixture();
    let settings = support::settings(&EXACT);
    let counted = support::count(&fixture, &plain(&images.join(",")), &settings, &fetcher)
        .await
        .unwrap();
    // 300 low-detail images count the 85 base tokens each and are never fetched
    assert_eq!(counted.tokens, 25_500);
    assert_eq!(fetcher.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn default_count_never_reaches_the_cap() {
    let urls = distinct_urls(300);
    let fetcher = MappingFetcher {
        urls: mapping(&urls),
        calls: AtomicUsize::new(0),
    };
    let fixture = support::words_fixture();
    let settings = support::settings(&DEFAULT);
    let counted = support::count(
        &fixture,
        &image_content(&urls, Layout::Flat),
        &settings,
        &fetcher,
    )
    .await
    .unwrap();
    // 300 default-count images count the 250 configured tokens each and are never fetched
    assert_eq!(counted.tokens, 75_000);
    assert_eq!(fetcher.calls.load(Ordering::SeqCst), 0);
}
