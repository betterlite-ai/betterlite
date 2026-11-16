use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};

use rstest::rstest;

use crate::media::count::Warning;
use crate::media::settings::ImageCounting;

use super::support::{
    self, CaseOptions, MappingFetcher, RefusingFetcher, image_url, image_url_string, plain,
};

// 1024x1024: resized to 768x768 that is four 512 tiles and 765 tokens, so counting the 300x300 default
// dimensions after the fetch (255) fails the assertions instead of passing them
const PNG: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAABAAAAAQA";
// what the downloader returns for a 200 that declares image/tiff: a non-generic type the dimension parser
// cannot place, unlike a generic or missing type, which the downloader refuses as UnknownMediaType
const UNKNOWN: &str = "data:image/tiff;base64,AAAA";
const REMOTE: &str = "https://example.com/image.png";
const DISABLED: &str = "Error: Image URL download is disabled (MAX_IMAGE_URL_DOWNLOAD_SIZE_MB=0). url=https://example.com/image.png";
const HIGH_IMAGE: &str =
    r#"{"type":"image_url","image_url":{"url":"https://example.com/image.png","detail":"high"}}"#;

const EXACT: CaseOptions = CaseOptions {
    counting: ImageCounting::Exact,
    fallback: None,
    image_default_count: 250,
};
const EXACT_FALLBACK: CaseOptions = CaseOptions {
    counting: ImageCounting::Exact,
    fallback: Some(777),
    image_default_count: 250,
};
const DEFAULT: CaseOptions = CaseOptions {
    counting: ImageCounting::DefaultCount,
    fallback: None,
    image_default_count: 250,
};
const DEFAULT_123: CaseOptions = CaseOptions {
    counting: ImageCounting::DefaultCount,
    fallback: None,
    image_default_count: 123,
};

#[rstest]
#[case::string_image_url(image_url_string(REMOTE), EXACT, 85, 0)]
#[case::detail_low(image_url(REMOTE, "low"), EXACT, 85, 0)]
#[case::detail_auto(image_url(REMOTE, "auto"), EXACT, 85, 0)]
#[case::default_count(image_url(REMOTE, "high"), DEFAULT, 250, 1)]
#[case::default_count_settings(image_url(REMOTE, "high"), DEFAULT_123, 123, 1)]
#[tokio::test]
async fn remote_never_fetches_without_high_exact(
    #[case] input: String,
    #[case] options: CaseOptions,
    #[case] tokens: u64,
    #[case] remote_warnings: usize,
) {
    let fixture = support::words_fixture();
    let settings = support::settings(&options);
    let fetcher = RefusingFetcher {
        calls: AtomicUsize::new(0),
    };
    let counted = support::count(&fixture, &input, &settings, &fetcher)
        .await
        .unwrap();
    assert_eq!(counted.tokens, tokens);
    let remote = counted
        .warnings
        .iter()
        .filter(|warning| matches!(warning, Warning::RemoteImageDefaultCount))
        .count();
    assert_eq!(remote, remote_warnings);
    assert_eq!(fetcher.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn remote_high_exact_fetches_the_url() {
    let fixture = support::words_fixture();
    let settings = support::settings(&EXACT);
    let fetcher = MappingFetcher {
        urls: BTreeMap::from([(REMOTE.to_owned(), PNG.to_owned())]),
        calls: AtomicUsize::new(0),
    };
    let counted = support::count(&fixture, &image_url(REMOTE, "high"), &settings, &fetcher)
        .await
        .unwrap();
    assert_eq!(counted.tokens, 765);
    assert_eq!(fetcher.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn remote_high_exact_fetches_default_dimensions_for_unknown_bytes() {
    let fixture = support::words_fixture();
    let settings = support::settings(&EXACT);
    let fetcher = MappingFetcher {
        urls: BTreeMap::from([(REMOTE.to_owned(), UNKNOWN.to_owned())]),
        calls: AtomicUsize::new(0),
    };
    let counted = support::count(&fixture, &image_url(REMOTE, "high"), &settings, &fetcher)
        .await
        .unwrap();
    // the mapped bytes name no image, so this can only be the 300x300 defaults: PNG's parsed 1024x1024
    // would count 765, and a generic or missing declared type never reaches the parser at all
    assert_eq!(counted.tokens, 255);
    assert_eq!(fetcher.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn remote_high_exact_refusal_is_the_fetch_error() {
    let fixture = support::words_fixture();
    let settings = support::settings(&EXACT);
    let fetcher = RefusingFetcher {
        calls: AtomicUsize::new(0),
    };
    let error = support::count(&fixture, &image_url(REMOTE, "high"), &settings, &fetcher)
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), DISABLED);
    assert_eq!(fetcher.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn remote_high_exact_refusal_uses_the_fallback_count() {
    let fixture = support::words_fixture();
    let settings = support::settings(&EXACT_FALLBACK);
    let fetcher = RefusingFetcher {
        calls: AtomicUsize::new(0),
    };
    let counted = support::count(&fixture, &image_url(REMOTE, "high"), &settings, &fetcher)
        .await
        .unwrap();
    assert_eq!(counted.tokens, 777);
    let fallbacks = counted
        .warnings
        .iter()
        .filter(|warning| matches!(warning, Warning::FallbackTokenCount { .. }))
        .count();
    assert_eq!(fallbacks, 1);
    assert_eq!(fetcher.calls.load(Ordering::SeqCst), 1);
}

#[rstest]
#[case::tool_result(plain(&format!(
    r#"{{"type":"tool_result","tool_use_id":"toolu_1","content":[{HIGH_IMAGE}]}}"#
)))]
#[case::document(plain(&format!(
    r#"{{"type":"document","source":{{"type":"content","content":[{HIGH_IMAGE}]}}}}"#
)))]
#[tokio::test]
async fn remote_high_exact_fetches_inside_nested_content(#[case] input: String) {
    let fixture = support::words_fixture();
    let settings = support::settings(&EXACT);
    let fetcher = MappingFetcher {
        urls: BTreeMap::from([(REMOTE.to_owned(), PNG.to_owned())]),
        calls: AtomicUsize::new(0),
    };
    let counted = support::count(&fixture, &input, &settings, &fetcher)
        .await
        .unwrap();
    assert_eq!(counted.tokens, 765);
    assert_eq!(fetcher.calls.load(Ordering::SeqCst), 1);
}

#[rstest]
#[case::image_source_url(plain(&format!(
    r#"{{"type":"image","source":{{"type":"url","url":"{REMOTE}"}}}}"#
)))]
#[case::document_source_url(plain(&format!(
    r#"{{"type":"document","source":{{"type":"url","url":"{REMOTE}"}}}}"#
)))]
#[case::file_data_url(plain(&format!(
    r#"{{"type":"file","file":{{"file_data":"{REMOTE}"}}}}"#
)))]
#[tokio::test]
async fn remote_other_sources_never_fetch(#[case] input: String) {
    let fixture = support::words_fixture();
    let settings = support::settings(&EXACT);
    let fetcher = RefusingFetcher {
        calls: AtomicUsize::new(0),
    };
    let counted = support::count(&fixture, &input, &settings, &fetcher)
        .await
        .unwrap();
    assert_eq!(counted.tokens, 85);
    assert_eq!(fetcher.calls.load(Ordering::SeqCst), 0);
}
