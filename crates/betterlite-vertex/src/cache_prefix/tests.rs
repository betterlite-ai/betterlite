#![cfg(test)]

use std::ffi::OsStr;
use std::fs;
use std::future::{Future, ready};
use std::io;
use std::path::Path;

use betterlite_prompts::arguments::json::Json;
use betterlite_prompts::media::inline::RemoteMediaFetcher;
use betterlite_prompts::media_fetch::error::ImageFetchError;
use betterlite_prompts::message_content::Message;
use jiff::Timestamp;
use rstest::rstest;
use tokio::runtime::Builder;

use crate::cache_prefix::marking::is_cached_message;
use crate::cache_prefix::selection::{ends_on_supported_turn, separate_cached_messages};
use crate::cache_prefix::transform::{CachedContentRequest, cached_content_request};
use crate::cache_prefix::ttl::{extract_cached_ttl, parse_cache_ttl};
use crate::model_policy::capabilities::{
    CapabilityKind, CapabilityLookupError, ModelCapabilityLookup, VertexCapabilityProvider,
};

const NOW_SECOND: i64 = 1_794_441_600;

fn now() -> Timestamp {
    Timestamp::from_second(NOW_SECOND).unwrap()
}

fn message(source: &str) -> Message {
    serde_json::from_str(source).unwrap()
}

fn messages(source: &str) -> Vec<Message> {
    serde_json::from_str(source).unwrap()
}

fn json(source: &str) -> Json {
    serde_json::from_str(source).unwrap()
}

fn encode<T>(value: &T) -> Json
where
    T: serde::Serialize,
{
    serde_json::from_value(serde_json::to_value(value).unwrap()).unwrap()
}

#[rstest]
#[case::message_level(
    r#"{"role":"user","content":"t","cache_control":{"type":"ephemeral"}}"#,
    true
)]
#[case::text_part(
    r#"{"role":"user","content":[{"type":"text","text":"t","cache_control":{"type":"ephemeral"}}]}"#,
    true
)]
#[case::system_text_part(
    r#"{"role":"system","content":[{"type":"text","text":"t","cache_control":{"type":"ephemeral"}}]}"#,
    true
)]
#[case::image_part(
    r#"{"role":"user","content":[{"type":"image_url","image_url":{"url":"u"},"cache_control":{"type":"ephemeral"}}]}"#,
    false
)]
#[case::persistent_part(
    r#"{"role":"user","content":[{"type":"text","text":"t","cache_control":{"type":"persistent"}}]}"#,
    false
)]
#[case::string_content(r#"{"role":"user","content":"t"}"#, false)]
#[case::absent_content(r#"{"role":"user"}"#, false)]
#[case::null_content(r#"{"role":"user","content":null}"#, false)]
#[case::non_object_cache_control(
    r#"{"role":"user","content":[{"type":"text","text":"t","cache_control":"ephemeral"}]}"#,
    false
)]
fn marks_messages_with_an_ephemeral_marker(#[case] source: &str, #[case] expected: bool) {
    assert_eq!(is_cached_message(&message(source), false), expected);
}

#[test]
fn disables_marking() {
    let marked = message(r#"{"role":"user","content":"t","cache_control":{"type":"ephemeral"}}"#);
    assert!(is_cached_message(&marked, false));
    assert!(!is_cached_message(&marked, true));
}

#[test]
fn separates_the_first_continuous_run() {
    let parsed = messages(
        r#"[{"role":"system","content":"s"},{"role":"user","content":"a","cache_control":{"type":"ephemeral"}},{"role":"user","content":"b","cache_control":{"type":"ephemeral"}},{"role":"assistant","content":"c"},{"role":"user","content":"d","cache_control":{"type":"ephemeral"}}]"#,
    );
    let (cached, non_cached) = separate_cached_messages(&parsed, false);
    assert_eq!(
        encode(&cached),
        json(
            r#"[{"role":"user","content":"a","cache_control":{"type":"ephemeral"}},{"role":"user","content":"b","cache_control":{"type":"ephemeral"}}]"#
        )
    );
    assert_eq!(
        encode(&non_cached),
        json(
            r#"[{"role":"system","content":"s"},{"role":"assistant","content":"c"},{"role":"user","content":"d","cache_control":{"type":"ephemeral"}}]"#
        )
    );
}

#[test]
fn separates_nothing_without_markers() {
    let parsed = messages(r#"[{"role":"user","content":"a"},{"role":"assistant","content":"b"}]"#);
    let (cached, non_cached) = separate_cached_messages(&parsed, false);
    assert!(cached.is_empty());
    assert_eq!(encode(&non_cached), encode(&parsed));
}

#[test]
fn separates_nothing_when_the_transform_is_disabled() {
    let parsed =
        messages(r#"[{"role":"user","content":"a","cache_control":{"type":"ephemeral"}}]"#);
    let (cached, non_cached) = separate_cached_messages(&parsed, true);
    assert!(cached.is_empty());
    assert_eq!(encode(&non_cached), encode(&parsed));
}

#[test]
fn keeps_a_later_marked_run_after_the_first() {
    let parsed = messages(
        r#"[{"role":"user","content":"a","cache_control":{"type":"ephemeral"}},{"role":"assistant","content":"b","cache_control":{"type":"ephemeral"}},{"role":"user","content":"c"},{"role":"user","content":"d","cache_control":{"type":"ephemeral"}}]"#,
    );
    let (cached, non_cached) = separate_cached_messages(&parsed, false);
    assert_eq!(
        encode(&cached),
        json(
            r#"[{"role":"user","content":"a","cache_control":{"type":"ephemeral"}},{"role":"assistant","content":"b","cache_control":{"type":"ephemeral"}}]"#
        )
    );
    assert_eq!(
        encode(&non_cached),
        json(
            r#"[{"role":"user","content":"c"},{"role":"user","content":"d","cache_control":{"type":"ephemeral"}}]"#
        )
    );
}

#[test]
fn all_marked_messages_form_one_run() {
    let parsed = messages(
        r#"[{"role":"user","content":"a","cache_control":{"type":"ephemeral"}},{"role":"user","content":"b","cache_control":{"type":"ephemeral"}}]"#,
    );
    let (cached, non_cached) = separate_cached_messages(&parsed, false);
    assert_eq!(cached.len(), 2);
    assert!(non_cached.is_empty());
}

#[rstest]
#[case::user(r#"[{"role":"user","content":"a"}]"#, true)]
#[case::assistant(
    r#"[{"role":"user","content":"a"},{"role":"assistant","content":"b"}]"#,
    false
)]
#[case::tool(
    r#"[{"role":"user","content":"a"},{"role":"tool","tool_call_id":"c","content":"b"}]"#,
    false
)]
#[case::function(r#"[{"role":"function","name":"f","content":"b"}]"#, false)]
#[case::assistant_before_system(
    r#"[{"role":"assistant","content":"b"},{"role":"system","content":"s"}]"#,
    false
)]
#[case::user_before_system(
    r#"[{"role":"user","content":"a"},{"role":"system","content":"s"}]"#,
    true
)]
#[case::system_only(
    r#"[{"role":"system","content":"s"},{"role":"system","content":"t"}]"#,
    true
)]
#[case::empty("[]", false)]
fn accepts_only_runs_that_do_not_end_on_a_model_turn(#[case] source: &str, #[case] expected: bool) {
    assert_eq!(ends_on_supported_turn(&messages(source)), expected);
}

#[rstest]
#[case::first_valid(
    r#"[{"role":"system","content":[{"type":"text","text":"s","cache_control":{"type":"ephemeral","ttl":"invalid"}}]},{"role":"user","content":[{"type":"text","text":"a","cache_control":{"type":"ephemeral","ttl":"5m"}}]},{"role":"user","content":[{"type":"text","text":"b","cache_control":{"type":"ephemeral","ttl":"1h"}}]}]"#,
    Some("300s")
)]
#[case::second_part(
    r#"[{"role":"user","content":[{"type":"text","text":"a","cache_control":{"type":"ephemeral","ttl":"0s"}},{"type":"text","text":"b","cache_control":{"type":"ephemeral","ttl":"1.5s"}}]}]"#,
    Some("1.5s")
)]
#[case::message_level_ttl(
    r#"[{"role":"user","content":"a","cache_control":{"type":"ephemeral","ttl":"1h"}}]"#,
    None
)]
#[case::unmarked_ttl(
    r#"[{"role":"user","content":[{"type":"text","text":"a","cache_control":{"type":"ephemeral"}}]},{"role":"user","content":[{"type":"text","text":"b","cache_control":{"type":"persistent","ttl":"1h"}}]}]"#,
    None
)]
fn extracts_the_first_valid_ttl(#[case] source: &str, #[case] expected: Option<&str>) {
    assert_eq!(
        extract_cached_ttl(&messages(source), false, now()).as_deref(),
        expected
    );
}

#[test]
fn extracts_no_ttl_when_the_transform_is_disabled() {
    let parsed = messages(
        r#"[{"role":"user","content":[{"type":"text","text":"a","cache_control":{"type":"ephemeral","ttl":"1h"}}]}]"#,
    );
    assert_eq!(extract_cached_ttl(&parsed, true, now()), None);
}

#[rstest]
#[case::seconds("3600s", Some("3600s"))]
#[case::one_second("1s", Some("1s"))]
#[case::half_second("1.5s", Some("1.5s"))]
#[case::tenth("0.1s", Some("0.1s"))]
#[case::milliseconds("123.456s", Some("123.456s"))]
#[case::rounded("1.3333333333333333s", Some("1.333333333s"))]
#[case::minutes("5m", Some("300s"))]
#[case::ninety_minutes("90m", Some("5400s"))]
#[case::hour("1h", Some("3600s"))]
#[case::half_hour("0.5h", Some("1800s"))]
#[case::two_days("48h", Some("172800s"))]
#[case::long_hours("61320000h", Some("220752000000s"))]
#[case::leading_dot(".5s", Some("0.5s"))]
#[case::rounds_up("0.0000000006s", Some("0.000000001s"))]
#[case::horizon_limit("251607859199s", Some("251607859199s"))]
#[case::below_horizon("251607859198.5s", Some("251607859198.5s"))]
fn normalizes_supported_ttls(#[case] ttl: &str, #[case] expected: Option<&str>) {
    assert_eq!(parse_cache_ttl(ttl.as_bytes(), now()).as_deref(), expected);
}

#[rstest]
#[case::no_unit("3600")]
#[case::unit_only("s")]
#[case::negative("-1s")]
#[case::plus("+1s")]
#[case::zero("0s")]
#[case::zero_minutes("0m")]
#[case::zero_hours("0h")]
#[case::days("5d")]
#[case::letters("abc.s")]
#[case::empty("")]
#[case::dot_only(".")]
#[case::trailing_dot("3600.s")]
#[case::inner_space("3600 s")]
#[case::double_unit("3600ss")]
#[case::unit_space("1 h")]
#[case::under_nine_places("0.0000000001s")]
#[case::past_horizon("251607859200s")]
#[case::past_horizon_hours("69920000h")]
#[case::exponent("1e3s")]
#[case::trailing_newline("3600s\n")]
#[case::underscore("1_0s")]
#[case::non_ascii("\u{663}s")]
fn rejects_malformed_ttls(#[case] ttl: &str) {
    assert_eq!(parse_cache_ttl(ttl.as_bytes(), now()), None);
}

#[test]
fn rejects_ttls_that_are_not_utf8() {
    assert_eq!(parse_cache_ttl(&[0xff, b's'], now()), None);
}

#[test]
fn horizon_follows_the_injected_clock() {
    let later = Timestamp::from_second(1_794_441_601).unwrap();
    assert_eq!(
        parse_cache_ttl(b"251607859199s", now()).as_deref(),
        Some("251607859199s")
    );
    assert_eq!(parse_cache_ttl(b"251607859199s", later), None);
    assert_eq!(
        parse_cache_ttl(b"251607859198s", later).as_deref(),
        Some("251607859198s")
    );
}

// LiteLLM rounds a binary float, so an exact decimal tie at the tenth place can land on another nanosecond
#[test]
fn rounds_a_decimal_tie_to_the_even_place() {
    assert_eq!(parse_cache_ttl(b"0.0000000005s", now()), None);
    assert_eq!(
        parse_cache_ttl(b"1.9999999995s", now()).as_deref(),
        Some("2s")
    );
}

#[test]
fn replays_the_cache_ttl_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/cache_ttl");
    fs::read_dir(corpus)
        .expect("corpus folder")
        .try_for_each(|entry| {
            let path = entry?.path();
            let bytes = fs::read(&path)?;
            let name = path
                .file_name()
                .and_then(OsStr::to_str)
                .expect("utf-8 file name");
            let normalized = parse_cache_ttl(&bytes, now());
            if name.starts_with("valid") {
                assert!(normalized.is_some(), "{name}");
            } else {
                assert!(normalized.is_none(), "{name}");
            }
            Ok::<(), io::Error>(())
        })
        .expect("readable corpus");
}

struct NoFetch;

impl RemoteMediaFetcher for NoFetch {
    fn convert_url(&self, url: &str) -> impl Future<Output = Result<String, ImageFetchError>> {
        ready(Err(ImageFetchError::Attempts {
            url: url.to_owned(),
        }))
    }

    fn convert_urls(
        &self,
        urls: &[String],
    ) -> impl Future<Output = Result<Vec<String>, ImageFetchError>> {
        let url = urls.first().cloned().unwrap_or_default();
        ready(Err(ImageFetchError::Attempts { url }))
    }
}

struct FixtureLookup {
    supported: bool,
}

impl ModelCapabilityLookup for FixtureLookup {
    fn supports(
        &self,
        kind: CapabilityKind,
        _model: &str,
        _provider: &str,
    ) -> Result<bool, CapabilityLookupError> {
        match kind {
            CapabilityKind::SystemMessages => Ok(self.supported),
            CapabilityKind::ResponseSchema => Ok(false),
        }
    }
}

fn build_request<'request>(
    model: &'request str,
    message_list: &'request [Message],
    provider: VertexCapabilityProvider,
) -> CachedContentRequest<'request> {
    CachedContentRequest {
        model,
        messages: message_list,
        provider,
        cache_key: "cache-key",
        project: "proj",
        location: "us-central1",
        disable_transform: false,
        now: now(),
    }
}

fn cached_body(input: &CachedContentRequest<'_>, supported: bool) -> Json {
    let runtime = Builder::new_current_thread().build().unwrap();
    let lookup = FixtureLookup { supported };
    let built = runtime
        .block_on(cached_content_request(input, &lookup, &NoFetch))
        .unwrap();
    encode(&built)
}

#[test]
fn builds_the_vertex_cached_content_body() {
    let parsed = messages(
        r#"[{"role":"system","content":[{"type":"text","text":"rules","cache_control":{"type":"ephemeral","ttl":"1h"}}]},{"role":"user","content":[{"type":"text","text":"question","cache_control":{"type":"ephemeral"}}]}]"#,
    );
    let input = build_request(
        "gemini-2.5-pro",
        &parsed,
        VertexCapabilityProvider::VertexAi,
    );
    assert_eq!(
        cached_body(&input, true),
        json(
            r#"{"contents":[{"role":"user","parts":[{"text":"question"}]}],"model":"projects/proj/locations/us-central1/publishers/google/models/gemini-2.5-pro","displayName":"cache-key","ttl":"3600s","system_instruction":{"parts":[{"text":"rules"}]}}"#
        )
    );
}

#[test]
fn builds_the_vertex_beta_cached_content_body() {
    let parsed = messages(r#"[{"role":"user","content":"question"}]"#);
    let input = build_request(
        "gemini-2.5-pro",
        &parsed,
        VertexCapabilityProvider::VertexAiBeta,
    );
    assert_eq!(
        cached_body(&input, true),
        json(
            r#"{"contents":[{"role":"user","parts":[{"text":"question"}]}],"model":"projects/proj/locations/us-central1/publishers/google/models/gemini-2.5-pro","displayName":"cache-key"}"#
        )
    );
}

#[test]
fn builds_the_studio_cached_content_body() {
    let parsed = messages(r#"[{"role":"user","content":"question"}]"#);
    let input = build_request("gemini-2.5-pro", &parsed, VertexCapabilityProvider::Gemini);
    assert_eq!(
        cached_body(&input, true),
        json(
            r#"{"contents":[{"role":"user","parts":[{"text":"question"}]}],"model":"models/gemini-2.5-pro","displayName":"cache-key"}"#
        )
    );
}

#[test]
fn a_system_only_cached_request_gains_a_user_period() {
    let parsed = messages(
        r#"[{"role":"system","content":[{"type":"text","text":"rules","cache_control":{"type":"ephemeral"}}]}]"#,
    );
    let input = build_request(
        "gemini-2.5-pro",
        &parsed,
        VertexCapabilityProvider::VertexAi,
    );
    assert_eq!(
        cached_body(&input, true),
        json(
            r#"{"contents":[{"role":"user","parts":[{"text":"."}]}],"model":"projects/proj/locations/us-central1/publishers/google/models/gemini-2.5-pro","displayName":"cache-key","system_instruction":{"parts":[{"text":"rules"}]}}"#
        )
    );
}

#[test]
fn an_unsupported_system_message_stays_in_the_contents() {
    let parsed = messages(
        r#"[{"role":"system","content":[{"type":"text","text":"rules","cache_control":{"type":"ephemeral"}}]},{"role":"user","content":"question"}]"#,
    );
    let input = build_request("text-bison", &parsed, VertexCapabilityProvider::VertexAi);
    assert_eq!(
        cached_body(&input, false),
        json(
            r#"{"contents":[{"role":"user","parts":[{"text":"rules"},{"text":"question"}]}],"model":"projects/proj/locations/us-central1/publishers/google/models/text-bison","displayName":"cache-key"}"#
        )
    );
}

#[test]
fn an_invalid_ttl_is_omitted_from_the_body() {
    let parsed = messages(
        r#"[{"role":"user","content":[{"type":"text","text":"a","cache_control":{"type":"ephemeral","ttl":"invalid"}}]}]"#,
    );
    let input = build_request(
        "gemini-2.5-pro",
        &parsed,
        VertexCapabilityProvider::VertexAi,
    );
    assert_eq!(
        cached_body(&input, true),
        json(
            r#"{"contents":[{"role":"user","parts":[{"text":"a"}]}],"model":"projects/proj/locations/us-central1/publishers/google/models/gemini-2.5-pro","displayName":"cache-key"}"#
        )
    );
}

#[test]
fn a_disabled_transform_omits_the_ttl() {
    let parsed = messages(
        r#"[{"role":"system","content":[{"type":"text","text":"rules","cache_control":{"type":"ephemeral","ttl":"1h"}}]},{"role":"user","content":"question"}]"#,
    );
    let mut input = build_request(
        "gemini-2.5-pro",
        &parsed,
        VertexCapabilityProvider::VertexAi,
    );
    input.disable_transform = true;
    assert_eq!(
        cached_body(&input, true),
        json(
            r#"{"contents":[{"role":"user","parts":[{"text":"question"}]}],"model":"projects/proj/locations/us-central1/publishers/google/models/gemini-2.5-pro","displayName":"cache-key","system_instruction":{"parts":[{"text":"rules"}]}}"#
        )
    );
}
