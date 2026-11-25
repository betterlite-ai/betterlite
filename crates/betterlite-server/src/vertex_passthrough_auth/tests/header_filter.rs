use http::header::HeaderValue;
use indexmap::IndexMap;
use secrecy::SecretString;

use super::super::headers::{
    allowed_vertex_headers, is_vertex_count_tokens_route, vertex_upstream_headers,
};
use super::support::{headers, prepared_text, texts};

fn upstream(pairs: &[(&str, &str)]) -> IndexMap<String, SecretString> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), SecretString::from(*value)))
        .collect()
}

#[test]
fn only_the_two_allowed_headers_are_read() {
    let map = headers(&[
        ("Anthropic-Beta", "beta-1"),
        ("Content-Type", "application/json"),
        ("Authorization", "Bearer caller-token"),
        ("X-Goog-Api-Key", "google-key"),
    ]);
    let allowed = allowed_vertex_headers(&map);
    assert_eq!(texts(&allowed), vec!["anthropic-beta", "content-type"]);
    assert_eq!(prepared_text(&allowed, "anthropic-beta"), Some("beta-1"));
    assert_eq!(
        prepared_text(&allowed, "content-type"),
        Some("application/json")
    );
}

#[test]
fn a_request_without_the_allowed_headers_gives_an_empty_map() {
    let map = headers(&[("Authorization", "Bearer caller-token")]);
    assert!(allowed_vertex_headers(&map).is_empty());
}

#[test]
fn a_repeated_allowed_header_keeps_its_first_value() {
    let mut map = http::HeaderMap::new();
    map.append("content-type", HeaderValue::from_static("first"));
    map.append("content-type", HeaderValue::from_static("second"));
    let allowed = allowed_vertex_headers(&map);
    assert_eq!(prepared_text(&allowed, "content-type"), Some("first"));
}

#[test]
fn the_segment_before_the_first_colon_decides_the_count_tokens_route() {
    assert!(is_vertex_count_tokens_route(
        "v1/projects/p/locations/l/publishers/anthropic/models/count-tokens:rawPredict"
    ));
    assert!(is_vertex_count_tokens_route("count-tokens"));
    assert!(is_vertex_count_tokens_route("a/count-tokens:rawPredict"));
    assert!(is_vertex_count_tokens_route(
        "a/count-tokens:rawPredict:again"
    ));
}

#[test]
fn another_last_segment_is_not_the_count_tokens_route() {
    assert!(!is_vertex_count_tokens_route(
        "v1/projects/p/locations/l/publishers/anthropic/models/claude:rawPredict"
    ));
    assert!(!is_vertex_count_tokens_route(
        "v1/projects/p/locations/l/publishers/anthropic/models/claude:count-tokens:rawPredict"
    ));
    assert!(!is_vertex_count_tokens_route(
        "v1/count-tokens/models/claude:rawPredict"
    ));
    assert!(!is_vertex_count_tokens_route("count-tokensx:rawPredict"));
    assert!(!is_vertex_count_tokens_route("count-tokens:x/y"));
    assert!(!is_vertex_count_tokens_route(""));
}

#[test]
fn a_count_tokens_route_drops_anthropic_beta_and_keeps_the_rest() {
    let map = upstream(&[
        ("Anthropic-Beta", "beta-1"),
        ("content-type", "application/json"),
        ("Authorization", "Bearer proxy-token"),
    ]);
    let filtered = vertex_upstream_headers(
        "v1/projects/p/locations/l/publishers/anthropic/models/count-tokens:rawPredict",
        &map,
    );
    assert_eq!(texts(&filtered), vec!["content-type", "Authorization"]);
    assert_eq!(
        prepared_text(&filtered, "content-type"),
        Some("application/json")
    );
    assert_eq!(prepared_text(&map, "Anthropic-Beta"), Some("beta-1"));
}

#[test]
fn another_route_keeps_anthropic_beta() {
    let map = upstream(&[
        ("Anthropic-Beta", "beta-1"),
        ("content-type", "application/json"),
    ]);
    let kept = vertex_upstream_headers(
        "v1/projects/p/locations/l/publishers/anthropic/models/claude:rawPredict",
        &map,
    );
    assert_eq!(texts(&kept), vec!["Anthropic-Beta", "content-type"]);
    assert_eq!(prepared_text(&kept, "Anthropic-Beta"), Some("beta-1"));
}

#[test]
fn a_model_colon_count_tokens_route_keeps_anthropic_beta() {
    let map = upstream(&[
        ("Anthropic-Beta", "beta-1"),
        ("content-type", "application/json"),
    ]);
    let kept = vertex_upstream_headers(
        "v1/projects/p/locations/l/publishers/anthropic/models/claude:count-tokens:rawPredict",
        &map,
    );
    assert_eq!(texts(&kept), vec!["Anthropic-Beta", "content-type"]);
    assert_eq!(prepared_text(&kept, "Anthropic-Beta"), Some("beta-1"));
}
