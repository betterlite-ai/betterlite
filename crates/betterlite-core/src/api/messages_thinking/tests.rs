#![cfg(test)]

mod support;

use std::fs;
use std::iter::repeat_n;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_logging::passthrough_logging_state::logger::PassThroughEndpointLogging;
use betterlite_logging::worker::{LoggingWorker, WorkerSettings};
use betterlite_models::decisions::Json;
use betterlite_models::full_model_metadata::record::ModelInfo;
use betterlite_models::metadata::levels::ReasoningEffort;
use betterlite_models::model_metadata_resolution::names::ModelCostMap;
use indexmap::IndexMap;
use rstest::rstest;
use serde_json::value::RawValue;

use crate::api::api_boundaries::ApiLimits;
use crate::api::messages_stream_logging::StreamDeps;
use crate::api::messages_stream_logging::slots::StreamSlots;

use super::bridge::{BridgeKwargs, local_model_name};
use super::effort::{degrade_effort, normalize_reasoning_effort};
use super::response::{ResponseJsonError, decode_messages_response};
use super::session::{SessionKeyError, decode_messages_thinking};
use super::stream;
use super::summary::auto_summary_enabled;

const LIMITS: ApiLimits = ApiLimits {
    max_json_bytes: 4096,
    max_event_bytes: 4096,
    max_items: 100,
};

#[rstest]
#[case::claude_code(r#"{"session_id":"session_a","user_id":"person"}"#, Some("session_a"))]
#[case::plain_person("person", None)]
#[case::invalid_json("{\"session_id\":", None)]
#[case::empty_list("[]", None)]
#[case::empty_object("{}", None)]
#[case::session_id_number(r#"{"session_id":7}"#, None)]
#[case::session_id_null(r#"{"session_id":null}"#, None)]
#[case::empty_session_id(r#"{"session_id":""}"#, None)]
#[case::extra_metadata(
    r#"{"session_id":"session_a","user_id":{"id":7},"v":2}"#,
    Some("session_a")
)]
fn session_cache_key(#[case] body: &str, #[case] expected: Option<&str>) {
    let key = decode_messages_thinking(body.as_bytes(), &LIMITS).expect("the body fits the limit");
    assert_eq!(key.as_deref(), expected);
}

#[rstest]
#[case::ascii('a')]
#[case::unicode('\u{e9}')]
fn session_cache_key_truncates_at_64_characters(#[case] character: char) {
    let session: String = repeat_n(character, 65).collect();
    let body = format!(r#"{{"session_id":"{session}"}}"#);
    let key = decode_messages_thinking(body.as_bytes(), &LIMITS)
        .expect("the body fits the limit")
        .expect("the session becomes a key");
    let expected: String = repeat_n(character, 64).collect();
    assert_eq!(key, expected);
}

#[test]
fn session_cache_key_rejects_an_oversized_body() {
    let body = format!(
        r#"{{"session_id":"{}"}}"#,
        "a".repeat(LIMITS.max_json_bytes)
    );
    let error = decode_messages_thinking(body.as_bytes(), &LIMITS).expect_err("the body is over");
    assert!(matches!(error, SessionKeyError::Limit(_)));
}

#[rstest]
#[case::unchanged(200, r#"{"id":"msg_a","model":"provider","extra":7}"#)]
#[case::empty_object(200, "{}")]
#[case::error_status(
    429,
    r#"{"type":"error","error":{"type":"rate_limit_error","message":"slow"}}"#
)]
fn response_json_behavior(#[case] status: u16, #[case] body: &str) {
    let fields =
        decode_messages_response(body.as_bytes(), status, &LIMITS).expect("the response decodes");
    assert_eq!(
        serde_json::to_string(&fields).expect("the fields encode"),
        body
    );
}

#[rstest]
#[case::broken("broken")]
// Python's json reads these constants, and the wire format's standard JSON does not
#[case::nan(r#"{"value": NaN}"#)]
#[case::infinity(r#"{"value": Infinity}"#)]
#[case::negative_infinity(r#"{"value": -Infinity}"#)]
fn response_json_reports_invalid_json_as_an_anthropic_error(#[case] body: &str) {
    let error =
        decode_messages_response(body.as_bytes(), 502, &LIMITS).expect_err("the body is not JSON");
    match error {
        ResponseJsonError::Anthropic(anthropic) => {
            assert_eq!(anthropic.status_code, 502);
            assert_eq!(anthropic.message, body);
        }
        ResponseJsonError::Limit(_) | ResponseJsonError::NotAnObject => {
            panic!("the failure is the AnthropicError json() raises");
        }
    }
}

#[test]
fn response_json_names_a_body_that_is_not_utf8() {
    let error =
        decode_messages_response(b"\xff\xfe", 502, &LIMITS).expect_err("the body is not text");
    match error {
        ResponseJsonError::Anthropic(anthropic) => {
            assert_eq!(anthropic.status_code, 502);
            assert_eq!(anthropic.message, "2 bytes that are not UTF-8");
        }
        ResponseJsonError::Limit(_) | ResponseJsonError::NotAnObject => {
            panic!("a body that is not JSON is the AnthropicError json() raises");
        }
    }
}

#[rstest]
#[case::list("[]")]
#[case::null("null")]
#[case::text(r#""msg_a""#)]
#[case::number("7")]
fn response_json_rejects_a_non_object(#[case] body: &str) {
    let error = decode_messages_response(body.as_bytes(), 200, &LIMITS)
        .expect_err("a non-mapping body fails the expansion");
    assert!(matches!(&error, ResponseJsonError::NotAnObject));
    assert_eq!(error.status_code(), 500);
}

#[test]
fn response_json_rejects_an_oversized_body() {
    let body = "a".repeat(LIMITS.max_json_bytes.saturating_add(1));
    let error =
        decode_messages_response(body.as_bytes(), 200, &LIMITS).expect_err("over the limit");
    assert!(matches!(&error, ResponseJsonError::Limit(_)));
    assert_eq!(error.status_code(), 413);
}

#[rstest]
#[case::openai_prefix("openai/gpt-4o", Some("openai"), "gpt-4o")]
#[case::other_provider("vendor/openai/gpt-4o", Some("openai"), "vendor/openai/gpt-4o")]
#[case::double_prefix("openai/openai/gpt", Some("openai"), "openai/gpt")]
#[case::unprefixed("gpt-4o", Some("openai"), "gpt-4o")]
#[case::no_provider("openai/gpt-4o", None, "openai/gpt-4o")]
fn bridge_identity_helpers(
    #[case] model: &str,
    #[case] provider: Option<&str>,
    #[case] expected: &str,
) {
    assert_eq!(local_model_name(model, provider), expected);
}

#[test]
fn bridge_kwargs_keep_the_supplied_logging_object() {
    let logging_obj = support::logging_object();
    let kwargs = BridgeKwargs {
        litellm_logging_obj: Some(Arc::clone(&logging_obj)),
        request_body: IndexMap::new(),
    };
    let found = kwargs.litellm_logging_obj.as_ref();
    assert!(found.is_some_and(|held| Arc::ptr_eq(held, &logging_obj)));
    assert!(BridgeKwargs::default().litellm_logging_obj.is_none());
}

#[rstest]
#[case::max_to_high("max", vec![ReasoningEffort::High], "high")]
#[case::xhigh_to_high("xhigh", vec![ReasoningEffort::High], "high")]
#[case::max_to_xhigh("max", vec![ReasoningEffort::Xhigh, ReasoningEffort::High], "xhigh")]
#[case::minimal_to_low("minimal", vec![ReasoningEffort::Low], "low")]
#[case::max_to_medium("max", vec![ReasoningEffort::Medium], "medium")]
#[case::max_never_none("max", vec![ReasoningEffort::None], "high")]
#[case::xhigh_never_none("xhigh", vec![ReasoningEffort::None], "high")]
#[case::minimal_never_none("minimal", vec![ReasoningEffort::None], "low")]
fn reasoning_effort_degradation(
    #[case] effort: &str,
    #[case] supported: Vec<ReasoningEffort>,
    #[case] expected: &str,
) {
    assert_eq!(degrade_effort(effort, Some(supported.as_slice())), expected);
}

#[rstest]
#[case::max_floor("max", "high")]
#[case::xhigh_floor("xhigh", "high")]
#[case::minimal_floor("minimal", "low")]
fn reasoning_effort_degradation_without_metadata(#[case] effort: &str, #[case] expected: &str) {
    assert_eq!(degrade_effort(effort, None), expected);
    let empty: &[ReasoningEffort] = &[];
    assert_eq!(degrade_effort(effort, Some(empty)), expected);
}

#[rstest]
#[case::low("low", vec![ReasoningEffort::High], "low")]
#[case::medium("medium", vec![ReasoningEffort::High], "medium")]
#[case::high("high", vec![ReasoningEffort::Low], "high")]
#[case::none("none", vec![ReasoningEffort::High], "none")]
#[case::unknown("bogus", vec![ReasoningEffort::High], "bogus")]
fn reasoning_effort_keeps_levels_outside_the_chain(
    #[case] effort: &str,
    #[case] supported: Vec<ReasoningEffort>,
    #[case] expected: &str,
) {
    assert_eq!(degrade_effort(effort, Some(supported.as_slice())), expected);
}

fn model_info(fixture: &str) -> ModelInfo {
    let raw: Box<RawValue> = serde_json::from_str(fixture).expect("the fixture is JSON");
    ModelInfo::parse(&raw).expect("the fixture is a model info object")
}

#[rstest]
#[case::declared_high(
    r#"{"supports_reasoning":true,"reasoning_effort_levels":["high"]}"#,
    "max",
    "high"
)]
#[case::declared_xhigh(
    r#"{"supports_reasoning":true,"reasoning_effort_levels":["xhigh","high"]}"#,
    "max",
    "xhigh"
)]
#[case::declared_low(
    r#"{"supports_reasoning":true,"reasoning_effort_levels":["low"]}"#,
    "minimal",
    "low"
)]
#[case::declared_medium(
    r#"{"supports_reasoning":true,"reasoning_effort_levels":["medium"]}"#,
    "max",
    "medium"
)]
#[case::declared_none(
    r#"{"supports_reasoning":true,"reasoning_effort_levels":["none"]}"#,
    "max",
    "high"
)]
#[case::declared_empty(
    r#"{"supports_reasoning":true,"reasoning_effort_levels":[]}"#,
    "max",
    "high"
)]
#[case::unflagged(r#"{"supports_reasoning":true}"#, "max", "high")]
fn reasoning_effort_resolves_model_metadata(
    #[case] fixture: &str,
    #[case] effort: &str,
    #[case] expected: &str,
) {
    let model_cost = ModelCostMap::parse(b"{}").expect("an empty map parses");
    let info = model_info(fixture);
    assert_eq!(
        normalize_reasoning_effort(effort, Some(&info), &model_cost),
        expected
    );
}

#[test]
fn reasoning_effort_lookup_failure_keeps_the_floor() {
    let model_cost = ModelCostMap::parse(b"{}").expect("an empty map parses");
    assert_eq!(normalize_reasoning_effort("max", None, &model_cost), "high");
    assert_eq!(
        normalize_reasoning_effort("xhigh", None, &model_cost),
        "high"
    );
    assert_eq!(
        normalize_reasoning_effort("minimal", None, &model_cost),
        "low"
    );
}

#[rstest]
#[case::unset(false, None, false)]
#[case::global(true, None, true)]
#[case::env_true(false, Some("true"), true)]
#[case::env_upper_true(false, Some("TRUE"), true)]
#[case::env_one(false, Some("1"), false)]
#[case::env_yes(false, Some("yes"), false)]
#[case::env_padded(false, Some(" true "), false)]
#[case::global_true_env_false(true, Some("false"), true)]
#[case::global_false_env_upper_true(false, Some("TRUE"), true)]
fn loads_summary_settings(
    #[case] global: bool,
    #[case] environment: Option<&str>,
    #[case] expected: bool,
) {
    assert_eq!(auto_summary_enabled(global, environment), expected);
}

#[tokio::test]
async fn native_stream_factory() {
    let logging_obj = support::logging_object();
    let seams = Arc::new(support::Recorder {
        processed: Mutex::new(None),
        supplied: Arc::clone(&logging_obj),
    });
    let deps = StreamDeps {
        seams: Arc::clone(&seams),
        worker: Arc::new(
            LoggingWorker::start(WorkerSettings::default()).expect("the worker starts"),
        ),
        slots: Arc::new(StreamSlots::default()),
        clock: Arc::new(support::FixedClock),
        success_handler: Arc::new(PassThroughEndpointLogging {
            injected_log_dispatch: None,
        }),
    };
    let request_body = IndexMap::from([("model".to_owned(), Json::Text("claude".to_owned()))]);
    let relayed = stream::native_stream_factory(
        deps,
        "provider stream".to_owned(),
        request_body.clone(),
        Arc::clone(&logging_obj),
    );
    assert_eq!(relayed, "processed provider stream");
    let processed = seams
        .processed
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
        .expect("the factory reached the chunk processor");
    assert_eq!(processed.response, "provider stream");
    assert_eq!(processed.request_body, request_body);
    assert!(processed.same_logging_object);
    assert_eq!(processed.url_route, "/v1/messages");
    assert!(processed.endpoint_anthropic);
    assert_eq!(processed.start_time, support::time());
}

#[test]
fn replays_corpus() {
    let directory = concat!(env!("CARGO_MANIFEST_DIR"), "/fuzz/corpus/messages_thinking");
    let outcomes: Vec<bool> = fs::read_dir(directory)
        .expect("the corpus exists")
        .map(|entry| {
            let path = entry.expect("the entry lists").path();
            let bytes = fs::read(path).expect("the seed reads");
            let Some((&selector, payload)) = bytes.split_first() else {
                return false;
            };
            if selector.is_multiple_of(2) {
                decode_messages_response(payload, 500, &LIMITS).is_ok()
            } else {
                decode_messages_thinking(payload, &LIMITS).is_ok_and(|key| key.is_some())
            }
        })
        .collect();
    assert!(outcomes.contains(&true));
    assert!(outcomes.contains(&false));
}
