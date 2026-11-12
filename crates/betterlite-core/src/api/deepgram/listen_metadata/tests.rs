#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::str::from_utf8;
use std::sync::Arc;

use betterlite_models::catalog::{BundledCatalog, ModelCatalog, UnknownFields};
use rstest::rstest;
use rust_decimal::Decimal;

use super::error::ListenMetadataError;
use super::frames::{ListenFrame, audio_seconds, channel_count, transcript};
use super::pricing::{addon_pricing_models, is_priced, pricing_model, registry_key};
use super::query::{DEFAULT_MODEL, decode_listen_query};
use super::target::{ListenTarget, parse_listen_target, websocket_target};
use crate::api::deepgram::common::DeepgramException;

fn target(upstream_url: &str) -> ListenTarget {
    parse_listen_target(upstream_url).expect("a valid upstream listen URL")
}

fn frames(source: &str) -> Vec<ListenFrame> {
    serde_json::from_str(source).expect("typed listen frames")
}

fn bundled() -> Arc<ModelCatalog> {
    BundledCatalog::default().get()
}

#[rstest]
#[case::duplicates_and_blanks(b"model=&a=1&a=2&x=~y+z%7E", "model=&a=1&a=2&x=~y+z~")]
#[case::percent_encoded(b"model=nova-%C3%BC", "model=nova-%C3%BC")]
#[case::literal_utf8(b"model=nova-\xc3\xbc", "model=nova-%C3%BC")]
#[case::invalid_escape(b"a=%zz", "a=%25zz")]
#[case::without_value(b"flag&a=1", "flag=&a=1")]
fn decode_listen_query_keeps_wire_order_duplicates_and_blank_values(
    #[case] source: &[u8],
    #[case] encoded: &str,
) {
    assert_eq!(decode_listen_query(source).encode(), encoded);
}

#[rstest]
#[case::first("model=nova-2&model=nova-3", "nova-2")]
#[case::only("language=en&model=nova-3", "nova-3")]
#[case::blank_first("model=&model=nova-2", DEFAULT_MODEL)]
#[case::absent("language=en", DEFAULT_MODEL)]
fn requested_model_takes_the_first_value_or_the_default(
    #[case] query: &str,
    #[case] expected: &str,
) {
    let decoded = decode_listen_query(query.as_bytes());
    assert_eq!(decoded.requested_model(), expected);
}

#[test]
fn first_occurrences_keeps_only_the_first_model_and_language() {
    let decoded = decode_listen_query(
        b"model=nova-3&model=nova-2&language=en&language=multi&redact=true&redact=false",
    );
    let kept = decoded.first_occurrences();
    assert_eq!(kept.values("model"), vec!["nova-3"]);
    assert_eq!(kept.values("language"), vec!["en"]);
    assert_eq!(kept.values("redact"), vec!["true", "false"]);
    assert_eq!(
        kept.encode(),
        "model=nova-3&language=en&redact=true&redact=false"
    );
}

#[rstest]
#[case::default_base(
    None,
    "model=nova-3&language=en",
    "wss://api.deepgram.com/v1/listen?model=nova-3&language=en"
)]
#[case::https_base(
    Some("https://api.deepgram.com/v1"),
    "model=nova-3",
    "wss://api.deepgram.com/v1/listen?model=nova-3"
)]
#[case::trailing_slash(
    Some("https://api.deepgram.com/v1/"),
    "model=nova-3",
    "wss://api.deepgram.com/v1/listen?model=nova-3"
)]
#[case::http_base(
    Some("http://localhost:8080/base"),
    "model=nova-3",
    "ws://localhost:8080/base/listen?model=nova-3"
)]
#[case::websocket_base(
    Some("wss://example.test/ws"),
    "model=nova-3",
    "wss://example.test/ws/listen?model=nova-3"
)]
#[case::other_scheme(
    Some("ftp://files.test/a"),
    "model=nova-3",
    "ftp://files.test/a/listen?model=nova-3"
)]
#[case::empty_base(
    Some(""),
    "model=nova-3",
    "wss://api.deepgram.com/v1/listen?model=nova-3"
)]
fn websocket_target_builds_the_listen_url(
    #[case] api_base: Option<&str>,
    #[case] query: &str,
    #[case] expected: &str,
) {
    let decoded = decode_listen_query(query.as_bytes());
    let built = websocket_target(api_base, &decoded).expect("a valid base URL");
    assert_eq!(built, expected);
}

#[rstest]
#[case::blank_first(
    "model=&language=en",
    "wss://api.deepgram.com/v1/listen?language=en&model=nova-3"
)]
#[case::blank_and_later(
    "model=&model=nova-2&language=en",
    "wss://api.deepgram.com/v1/listen?language=en&model=nova-3"
)]
#[case::absent(
    "language=en",
    "wss://api.deepgram.com/v1/listen?language=en&model=nova-3"
)]
#[case::present("model=nova-2", "wss://api.deepgram.com/v1/listen?model=nova-2")]
fn websocket_target_replaces_a_blank_or_absent_model(#[case] query: &str, #[case] expected: &str) {
    let decoded = decode_listen_query(query.as_bytes());
    let built = websocket_target(None, &decoded).expect("the default base URL");
    assert_eq!(built, expected);
}

#[rstest]
#[case::bare_host("api.deepgram.com/v1")]
#[case::relative("/v1")]
#[case::no_host("://")]
fn websocket_target_rejects_a_base_that_is_not_an_absolute_url(#[case] api_base: &str) {
    let decoded = decode_listen_query(b"model=nova-3");
    let error = websocket_target(Some(api_base), &decoded).expect_err("an invalid base URL");
    assert!(matches!(error, ListenMetadataError::Base { .. }));
}

#[test]
fn websocket_target_drops_an_explicit_default_port() {
    let decoded = decode_listen_query(b"model=nova-3");
    let built =
        websocket_target(Some("https://api.deepgram.com:443/v1"), &decoded).expect("a valid base");
    assert_eq!(built, "wss://api.deepgram.com/v1/listen?model=nova-3");
}

#[rstest]
#[case::both("callback=&callback_method=POST&other=1", &["callback", "callback_method"])]
#[case::method_only("callback_method=x", &["callback_method"])]
#[case::duplicates("callback=1&callback=2", &["callback"])]
#[case::absent("callbackx=1", &[])]
fn callback_params_lists_each_present_callback_once(
    #[case] query: &str,
    #[case] expected: &[&str],
) {
    let decoded = decode_listen_query(query.as_bytes());
    assert_eq!(decoded.callback_params(), expected);
}

#[test]
fn parse_listen_target_drops_blank_query_values() {
    let parsed =
        target("wss://api.deepgram.com/v1/listen?model=&model=nova-2&language=&redact=true");
    assert_eq!(parsed.query.first("model"), Some("nova-2"));
    assert_eq!(parsed.query.first("language"), None);
    assert_eq!(parsed.query.values("redact"), vec!["true"]);
}

#[rstest]
#[case::absolute("https://api.deepgram.com/v1/listen?model=nova-3")]
#[case::websocket("wss://api.deepgram.com/v1/listen")]
fn parse_listen_target_accepts_an_absolute_url(#[case] upstream_url: &str) {
    drop(parse_listen_target(upstream_url).expect("a valid upstream URL"));
}

#[rstest]
#[case::relative("listen")]
#[case::garbage("://")]
fn parse_listen_target_rejects_a_url_without_a_scheme(#[case] upstream_url: &str) {
    let error = parse_listen_target(upstream_url).expect_err("an invalid upstream URL");
    assert!(matches!(error, ListenMetadataError::Target { .. }));
}

#[rstest]
#[case::plain("model=nova-3", "streaming/nova-3")]
#[case::multilingual("model=nova-3&language=multi", "streaming/nova-3-multilingual")]
#[case::multilingual_case("model=nova-3&language=MuLtI", "streaming/nova-3-multilingual")]
#[case::multilingual_spaces("model=nova-3&language=%20multi%20", "streaming/nova-3-multilingual")]
#[case::other_language("model=nova-3&language=en", "streaming/nova-3")]
#[case::absent_model("language=en", "streaming/nova-3")]
#[case::blank_model("model=&model=nova-2", "streaming/nova-2")]
#[case::blank_language_before_multi(
    "model=nova-3&language=&language=multi",
    "streaming/nova-3-multilingual"
)]
fn pricing_model_builds_the_streaming_registry_name(#[case] query: &str, #[case] expected: &str) {
    let upstream_url = format!("wss://api.deepgram.com/v1/listen?{query}");
    assert_eq!(pricing_model(&target(&upstream_url)), expected);
}

#[rstest]
#[case::plain("model=nova-3", "deepgram/streaming/nova-3")]
#[case::multilingual(
    "model=nova-3&language=multi",
    "deepgram/streaming/nova-3-multilingual"
)]
fn registry_key_prefixes_the_provider(#[case] query: &str, #[case] expected: &str) {
    let upstream_url = format!("wss://api.deepgram.com/v1/listen?{query}");
    assert_eq!(registry_key(&target(&upstream_url)), expected);
}

#[rstest]
#[case::streaming("model=nova-3", true)]
#[case::multilingual("model=nova-3&language=multi", true)]
#[case::prerecorded_only("model=nova-2", false)]
#[case::unknown("model=does-not-exist", false)]
#[case::different_case("model=NOVA-3", false)]
fn is_priced_requires_an_exact_streaming_entry(#[case] query: &str, #[case] expected: bool) {
    let catalog = bundled();
    let upstream_url = format!("wss://api.deepgram.com/v1/listen?{query}");
    assert_eq!(is_priced(&catalog, &target(&upstream_url)), expected);
}

#[test]
fn is_priced_rejects_an_entry_of_another_provider() {
    let source = r#"{"deepgram/streaming/nova-3": {"litellm_provider": "elsewhere"}}"#;
    let catalog = ModelCatalog::parse(source.as_bytes(), UnknownFields::Reject).expect("a catalog");
    let parsed = target("wss://api.deepgram.com/v1/listen?model=nova-3");
    assert!(!is_priced(&catalog, &parsed));
}

#[rstest]
#[case::mixed(
    "redact=true&keyterm=false&detect_entities=0&diarize_model=True",
    &["streaming/detect_entities", "streaming/diarize", "streaming/redact"]
)]
#[case::both_diarize("diarize=false&diarize_model=1", &["streaming/diarize"])]
#[case::disabled_forms(
    "redact=&keyterm=%20false%20&detect_entities=FALSE&diarize=no",
    &["streaming/diarize"]
)]
#[case::whitespace_only("detect_entities=%20%20", &[])]
#[case::any_enabled_duplicate("redact=false&redact=true", &["streaming/redact"])]
#[case::absent("model=nova-3", &[])]
fn addon_pricing_models_enables_each_enabled_addon_once(
    #[case] query: &str,
    #[case] expected: &[&str],
) {
    let upstream_url = format!("wss://api.deepgram.com/v1/listen?{query}");
    let wanted: Vec<String> = expected.iter().map(|name| (*name).to_owned()).collect();
    assert_eq!(addon_pricing_models(&target(&upstream_url)), wanted);
}

#[rstest]
#[case::present("channels=4", Some(4))]
#[case::padded("channels=007", Some(7))]
#[case::zero("channels=0", None)]
#[case::text("channels=abc", None)]
#[case::blank("channels=", None)]
#[case::non_ascii_digits("channels=%EF%BC%91", None)]
#[case::first_of_duplicates("channels=3&channels=2", Some(3))]
fn declared_channels_accepts_only_positive_ascii_digits(
    #[case] query: &str,
    #[case] expected: Option<u64>,
) {
    let decoded = decode_listen_query(query.as_bytes());
    assert_eq!(decoded.declared_channels(), expected);
}

#[test]
fn channel_count_uses_the_last_valid_metadata_count() {
    let frames = frames(
        r#"[
            {"type": "Metadata", "channels": 2},
            {"type": "Metadata", "channels": true},
            {"type": "Metadata", "channels": 0},
            {"type": "Metadata", "channels": -1},
            {"type": "Metadata", "channels": 4.0},
            {"type": "Metadata", "channels": 3},
            {"type": "Results", "channel_index": [0, 9]}
        ]"#,
    );
    assert_eq!(channel_count(&frames, Some(7)), 3);
}

#[test]
fn channel_count_takes_the_largest_two_element_results_index() {
    let frames = frames(
        r#"[
            {"type": "Metadata", "channels": true},
            {"type": "Results", "channel_index": [0, 3]},
            {"type": "Results", "channel_index": [0, 2]},
            {"type": "Results", "channel_index": [0, 1, 9]},
            {"type": "Results", "channel_index": [0]},
            {"type": "Results", "channel_index": [0, true]}
        ]"#,
    );
    assert_eq!(channel_count(&frames, Some(5)), 3);
}

#[rstest]
#[case::declared(Some(4), 4)]
#[case::absent(None, 1)]
fn channel_count_falls_back_to_declared_channels_then_one(
    #[case] declared: Option<u64>,
    #[case] expected: u64,
) {
    let frames = frames(
        r#"[
            {"type": "Metadata", "channels": -1},
            {"type": "Results", "channel_index": [0, 0]}
        ]"#,
    );
    assert_eq!(channel_count(&frames, declared), expected);
}

#[test]
fn audio_seconds_uses_the_last_positive_metadata_duration() {
    let frames = frames(
        r#"[
            {"type": "Metadata", "duration": 1.5},
            {"type": "Metadata", "duration": 0},
            {"type": "Metadata", "duration": -2.0},
            {"type": "Metadata", "duration": 2.5},
            {"type": "Results", "start": 8.0, "duration": 1.0}
        ]"#,
    );
    assert_eq!(audio_seconds(&frames), Decimal::new(25, 1));
}

#[test]
fn audio_seconds_uses_the_largest_results_end_without_a_metadata_duration() {
    let frames = frames(
        r#"[
            {"type": "Metadata", "duration": 0},
            {"type": "Results", "start": 1.5, "duration": 2.25},
            {"type": "Results", "start": 3.0, "duration": 0.5},
            {"type": "Results", "start": 5.0, "duration": -1.0},
            {"type": "Results", "start": -1.0, "duration": 1.0},
            {"type": "Results", "start": true, "duration": 9.0}
        ]"#,
    );
    assert_eq!(audio_seconds(&frames), Decimal::new(375, 2));
}

#[rstest]
#[case::empty("[]")]
#[case::rejected_metadata(
    r#"[{"type": "Metadata", "duration": 0}, {"type": "Metadata", "duration": -1}, {"type": "Results", "start": 1.0}]"#
)]
fn audio_seconds_is_zero_without_candidates(#[case] source: &str) {
    assert_eq!(audio_seconds(&frames(source)), Decimal::ZERO);
}

#[test]
fn audio_seconds_adds_results_bounds_as_decimals() {
    let frames = frames(r#"[{"type": "Results", "start": 0.1, "duration": 0.2}]"#);
    assert_eq!(audio_seconds(&frames), Decimal::new(3, 1));
}

#[test]
fn transcript_joins_final_first_alternatives_in_frame_order() {
    let frames = frames(
        r#"[
            {"type": "Results", "is_final": true, "channel": {"alternatives": [{"transcript": "hello"}, {"transcript": "ignored"}]}},
            {"type": "Results", "is_final": false, "channel": {"alternatives": [{"transcript": "not final"}]}},
            {"type": "Results", "is_final": true, "channel": {"alternatives": [{"transcript": ""}, {"transcript": "ignored"}]}},
            {"type": "Results", "is_final": true, "channel": {"alternatives": []}},
            {"type": "Results", "is_final": true},
            {"type": "Metadata", "transcript": "meta"},
            {"type": "Results", "is_final": true, "channel": {"alternatives": [{"transcript": "world"}]}}
        ]"#,
    );
    assert_eq!(transcript(&frames), "hello world");
}

#[test]
fn transcript_ignores_every_other_frame() {
    let frames =
        frames(r#"[{"type": "Metadata", "channel": {"alternatives": [{"transcript": "meta"}]}}]"#);
    assert_eq!(transcript(&frames), "");
}

#[test]
fn deepgram_exception_reports_the_provider_message() {
    let error = DeepgramException {
        status_code: 400,
        message: "invalid audio".to_owned(),
        headers: BTreeMap::new(),
    };
    assert_eq!(error.status_code, 400);
    assert_eq!(error.to_string(), "invalid audio");
}

#[rstest]
fn replays_the_listen_query_corpus(#[files("fuzz/corpus/listen_query/*")] path: PathBuf) {
    let bytes = fs::read(&path).expect("a corpus file");
    let query = decode_listen_query(&bytes);
    let _model: &str = query.requested_model();
    drop(query.first_occurrences());
    drop(query.callback_params());
    let _channels: Option<u64> = query.declared_channels();
    drop(query.without_blank_values());
    drop(query.encode());
    drop(websocket_target(None, &query));
    if let Ok(text) = from_utf8(&bytes) {
        drop(parse_listen_target(text));
    }
}
