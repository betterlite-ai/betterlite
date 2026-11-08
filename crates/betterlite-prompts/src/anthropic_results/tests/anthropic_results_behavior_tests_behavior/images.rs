use std::collections::BTreeMap;

use crate::anthropic_results::images::{
    ImageChunk, ImageError, create_anthropic_image_param, to_anthropic_image_chunk,
    to_openai_image_url,
};
use crate::arguments::json::Json;

use super::{MapSource, maps, object};

fn chunk(media_type: &str, data: &str) -> ImageChunk {
    ImageChunk {
        kind: "base64".to_owned(),
        media_type: media_type.to_owned(),
        data: data.to_owned(),
    }
}

fn input(raw: &str) -> Json {
    serde_json::from_str(raw).unwrap()
}

#[test]
fn anthropic_results_behavior_tests_behavior_an_image_chunk_renders_the_openai_url() {
    assert_eq!(
        to_openai_image_url(&chunk("image/jpeg", "AAA")),
        "data:image/jpeg;base64,AAA"
    );
}

#[tokio::test]
async fn anthropic_results_behavior_tests_behavior_a_data_url_is_split_into_its_chunk() {
    let urls = BTreeMap::new();
    let fetcher = MapSource { data_urls: &urls };
    let parsed = to_anthropic_image_chunk("data:image/png;base64,AAA", None, &fetcher)
        .await
        .unwrap();
    assert_eq!(parsed, chunk("image/png", "AAA"));
}

#[tokio::test]
async fn anthropic_results_behavior_tests_behavior_an_escaped_media_type_is_unescaped() {
    let urls = BTreeMap::new();
    let fetcher = MapSource { data_urls: &urls };
    let parsed = to_anthropic_image_chunk(r"data:image\/png;base64,AAA", None, &fetcher)
        .await
        .unwrap();
    assert_eq!(parsed, chunk("image/png", "AAA"));
}

#[tokio::test]
async fn anthropic_results_behavior_tests_behavior_a_given_format_wins_over_the_data_url() {
    let urls = BTreeMap::new();
    let fetcher = MapSource { data_urls: &urls };
    let parsed =
        to_anthropic_image_chunk("data:image/png;base64,AAA", Some("image/webp"), &fetcher)
            .await
            .unwrap();
    assert_eq!(parsed, chunk("image/webp", "AAA"));
}

#[tokio::test]
async fn anthropic_results_behavior_tests_behavior_an_empty_format_is_used_as_given() {
    let urls = BTreeMap::new();
    let fetcher = MapSource { data_urls: &urls };
    let parsed = to_anthropic_image_chunk("data:image/png;base64,AAA", Some(""), &fetcher)
        .await
        .unwrap();
    assert_eq!(parsed.media_type, "");
}

#[tokio::test]
async fn anthropic_results_behavior_tests_behavior_a_data_marker_after_a_prefix_is_used() {
    let urls = BTreeMap::new();
    let fetcher = MapSource { data_urls: &urls };
    let parsed = to_anthropic_image_chunk("prefixdata:image/png;base64,AAA", None, &fetcher)
        .await
        .unwrap();
    assert_eq!(parsed, chunk("image/png", "AAA"));
}

#[tokio::test]
async fn anthropic_results_behavior_tests_behavior_a_second_base64_marker_is_rejected() {
    let urls = BTreeMap::new();
    let fetcher = MapSource { data_urls: &urls };
    let error = to_anthropic_image_chunk("data:image/png;base64,AA;base64,BB", None, &fetcher)
        .await
        .unwrap_err();
    assert!(matches!(error, ImageError::DataUrl));
}

#[tokio::test]
async fn anthropic_results_behavior_tests_behavior_a_url_without_a_data_prefix_is_rejected() {
    let urls = BTreeMap::new();
    let fetcher = MapSource { data_urls: &urls };
    let error = to_anthropic_image_chunk("image/png;base64,AA", None, &fetcher)
        .await
        .unwrap_err();
    assert!(matches!(error, ImageError::DataUrl));
}

#[tokio::test]
async fn anthropic_results_behavior_tests_behavior_an_http_url_is_downloaded_first() {
    let urls = maps(&[("https://example.test/a.png", "data:image/png;base64,QUJD")]);
    let fetcher = MapSource { data_urls: &urls };
    let parsed = to_anthropic_image_chunk("https://example.test/a.png", None, &fetcher)
        .await
        .unwrap();
    assert_eq!(parsed, chunk("image/png", "QUJD"));
}

#[tokio::test]
async fn anthropic_results_behavior_tests_behavior_a_failed_download_keeps_its_fetch_error() {
    let urls = BTreeMap::new();
    let fetcher = MapSource { data_urls: &urls };
    let error = to_anthropic_image_chunk("https://example.test/missing.png", None, &fetcher)
        .await
        .unwrap_err();
    assert!(matches!(error, ImageError::Fetch(_)));
}

#[tokio::test]
async fn anthropic_results_behavior_tests_behavior_an_https_url_stays_a_url_source() {
    let urls = BTreeMap::new();
    let fetcher = MapSource { data_urls: &urls };
    let param = create_anthropic_image_param(
        &input(r#""https://example.test/a.png""#),
        None,
        false,
        &fetcher,
    )
    .await
    .unwrap();
    assert_eq!(
        param,
        object(r#"{"type":"image","source":{"type":"url","url":"https://example.test/a.png"}}"#)
    );
}

#[tokio::test]
async fn anthropic_results_behavior_tests_behavior_an_http_url_is_downloaded() {
    let urls = maps(&[("http://example.test/a.png", "data:image/png;base64,QUJD")]);
    let fetcher = MapSource { data_urls: &urls };
    let param = create_anthropic_image_param(
        &input(r#"{"url":"http://example.test/a.png"}"#),
        None,
        false,
        &fetcher,
    )
    .await
    .unwrap();
    assert_eq!(
        param,
        object(
            r#"{"type":"image","source":{"type":"base64","media_type":"image/png","data":"QUJD"}}"#
        )
    );
}

#[tokio::test]
async fn anthropic_results_behavior_tests_behavior_bedrock_invoke_downloads_an_https_url() {
    let urls = maps(&[("https://example.test/a.png", "data:image/png;base64,QUJD")]);
    let fetcher = MapSource { data_urls: &urls };
    let param = create_anthropic_image_param(
        &input(r#""https://example.test/a.png""#),
        None,
        true,
        &fetcher,
    )
    .await
    .unwrap();
    assert_eq!(
        param,
        object(
            r#"{"type":"image","source":{"type":"base64","media_type":"image/png","data":"QUJD"}}"#
        )
    );
}

#[tokio::test]
async fn anthropic_results_behavior_tests_behavior_an_object_fills_the_missing_format() {
    let urls = BTreeMap::new();
    let fetcher = MapSource { data_urls: &urls };
    let param = create_anthropic_image_param(
        &input(r#"{"url":"data:image/png;base64,QUJD","format":"image/webp"}"#),
        None,
        false,
        &fetcher,
    )
    .await
    .unwrap();
    assert_eq!(
        param,
        object(
            r#"{"type":"image","source":{"type":"base64","media_type":"image/webp","data":"QUJD"}}"#
        )
    );
}

#[tokio::test]
async fn anthropic_results_behavior_tests_behavior_a_given_format_beats_the_object_format() {
    let urls = BTreeMap::new();
    let fetcher = MapSource { data_urls: &urls };
    let param = create_anthropic_image_param(
        &input(r#"{"url":"data:image/png;base64,QUJD","format":"image/webp"}"#),
        Some("image/gif"),
        false,
        &fetcher,
    )
    .await
    .unwrap();
    assert_eq!(
        param,
        object(
            r#"{"type":"image","source":{"type":"base64","media_type":"image/gif","data":"QUJD"}}"#
        )
    );
}

#[tokio::test]
async fn anthropic_results_behavior_tests_behavior_an_object_without_a_url_fails_like_a_bad_data_url()
 {
    let urls = BTreeMap::new();
    let fetcher = MapSource { data_urls: &urls };
    let error =
        create_anthropic_image_param(&input(r#"{"format":"image/png"}"#), None, false, &fetcher)
            .await
            .unwrap_err();
    assert!(matches!(error, ImageError::DataUrl));
}

#[tokio::test]
async fn anthropic_results_behavior_tests_behavior_a_non_string_image_input_is_named() {
    let urls = BTreeMap::new();
    let fetcher = MapSource { data_urls: &urls };
    let error = create_anthropic_image_param(&input("5"), None, false, &fetcher)
        .await
        .unwrap_err();
    assert!(matches!(error, ImageError::Input));
}

#[tokio::test]
async fn anthropic_results_behavior_tests_behavior_a_non_string_url_is_named() {
    let urls = BTreeMap::new();
    let fetcher = MapSource { data_urls: &urls };
    let error = create_anthropic_image_param(&input(r#"{"url":5}"#), None, false, &fetcher)
        .await
        .unwrap_err();
    assert!(matches!(error, ImageError::Field { field: "url" }));
}

#[tokio::test]
async fn anthropic_results_behavior_tests_behavior_a_non_string_object_format_is_named() {
    let urls = BTreeMap::new();
    let fetcher = MapSource { data_urls: &urls };
    let error = create_anthropic_image_param(
        &input(r#"{"url":"data:image/png;base64,QUJD","format":5}"#),
        None,
        false,
        &fetcher,
    )
    .await
    .unwrap_err();
    assert!(matches!(error, ImageError::Field { field: "format" }));
}
