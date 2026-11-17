use betterlite_models::api_base_lookup::ApiBaseContext;
use betterlite_models::decisions::Json;
use chrono::TimeDelta;
use indexmap::IndexMap;
use rstest::rstest;
use serde_json::Number;

use crate::api::responses_stream_primitives::state::{
    BaseStreamStateInput, LoggingSnapshot, SharedResponsesApiBase, StreamTimes, base_stream_state,
};

use super::support::{
    FailingApiBase, NoApiBase, NoHooks, SharedResolution, logging_object, moment,
    resolution_context, snapshot, times,
};

fn metadata(pairs: &[(&str, Json)]) -> IndexMap<String, Json> {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect()
}

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

#[test]
fn base_stream_state_starts_with_the_source_defaults() {
    let headers = IndexMap::new();
    let state = base_stream_state(&BaseStreamStateInput {
        model: "gpt-4o",
        custom_llm_provider: Some("openai"),
        litellm_metadata: None,
        request_data: None,
        call_type: None,
        logging: snapshot(IndexMap::new()),
        headers: &headers,
        api_base: &NoApiBase,
        times: times(),
        max_streaming_duration_seconds: None,
    });
    assert_eq!(state.model, "gpt-4o");
    assert_eq!(state.custom_llm_provider.as_deref(), Some("openai"));
    assert_eq!(state.litellm_metadata, None);
    assert_eq!(state.call_type, None);
    assert!(!state.finished);
    assert!(!state.failure_handled);
    assert!(!state.yielded_first_chunk);
    assert!(!state.output_started);
    assert!(state.completed_response.is_none());
    assert!(!state.completed_response_cached);
    assert!(!state.completed_response_logged);
    assert_eq!(state.completed_response_cache_hit, None);
    assert!(state.persist_completed_response_before_logging);
    assert_eq!(state.generated_content, "");
    assert_eq!(state.generated_tool_arguments, "");
    assert!(state.pending_tasks.is_empty());
    assert_eq!(state.max_streaming_duration_seconds, None);
    assert_eq!(state.hidden_params.model_id, None);
    assert_eq!(state.hidden_params.api_base, None);
    assert_eq!(
        state.hidden_params.custom_llm_provider.as_deref(),
        Some("openai")
    );
}

#[test]
fn base_stream_state_turns_null_request_data_into_an_empty_object() {
    let headers = IndexMap::new();
    let empty = base_stream_state(&BaseStreamStateInput {
        model: "gpt-4o",
        custom_llm_provider: None,
        litellm_metadata: None,
        request_data: None,
        call_type: None,
        logging: snapshot(IndexMap::new()),
        headers: &headers,
        api_base: &NoApiBase,
        times: times(),
        max_streaming_duration_seconds: None,
    });
    assert!(empty.request_data.is_empty());
    let given = metadata(&[("stream", Json::Bool(true))]);
    let kept = base_stream_state(&BaseStreamStateInput {
        model: "gpt-4o",
        custom_llm_provider: None,
        litellm_metadata: None,
        request_data: Some(&given),
        call_type: Some("responses"),
        logging: snapshot(IndexMap::new()),
        headers: &headers,
        api_base: &NoApiBase,
        times: times(),
        max_streaming_duration_seconds: Some(2.5),
    });
    assert_eq!(kept.request_data.get("stream"), Some(&Json::Bool(true)));
    assert_eq!(kept.call_type.as_deref(), Some("responses"));
    assert_eq!(kept.max_streaming_duration_seconds, Some(2.5));
}

#[test]
fn base_stream_state_keeps_the_logging_start_time() {
    let headers = IndexMap::new();
    let state = base_stream_state(&BaseStreamStateInput {
        model: "gpt-4o",
        custom_llm_provider: None,
        litellm_metadata: None,
        request_data: None,
        call_type: None,
        logging: snapshot(IndexMap::new()),
        headers: &headers,
        api_base: &NoApiBase,
        times: times(),
        max_streaming_duration_seconds: None,
    });
    assert_eq!(state.start_time, moment());
    assert_eq!(state.created_timestamp, 0.0);
}

#[test]
fn base_stream_state_falls_back_to_now_without_a_logging_start_time() {
    let headers = IndexMap::new();
    let later = times().now + TimeDelta::seconds(7);
    let state = base_stream_state(&BaseStreamStateInput {
        model: "gpt-4o",
        custom_llm_provider: None,
        litellm_metadata: None,
        request_data: None,
        call_type: None,
        logging: LoggingSnapshot {
            start_time: None,
            litellm_params: IndexMap::new(),
        },
        headers: &headers,
        api_base: &NoApiBase,
        times: StreamTimes {
            now: later,
            created_timestamp: 12.5,
        },
        max_streaming_duration_seconds: None,
    });
    assert_eq!(state.start_time, later);
    assert_eq!(state.created_timestamp, 12.5);
}

#[test]
fn base_stream_state_reads_the_logging_snapshot() {
    let params = metadata(&[("api_base", text("https://logged.test"))]);
    let logging = logging_object(moment(), params);
    let read = LoggingSnapshot::read(&logging);
    assert_eq!(read.start_time, Some(moment()));
    assert_eq!(
        read.litellm_params.get("api_base"),
        Some(&text("https://logged.test"))
    );
}

#[rstest]
#[case::absent(None, None)]
#[case::empty_object(Some(IndexMap::new()), None)]
#[case::model_info_without_id(
    Some(metadata(&[("model_info", Json::Object(IndexMap::new()))])),
    None
)]
#[case::a_string_id(
    Some(metadata(&[(
        "model_info",
        Json::Object(metadata(&[("id", text("deployment-1"))]))
    )])),
    Some("deployment-1")
)]
#[case::an_empty_id_stays(
    Some(metadata(&[(
        "model_info",
        Json::Object(metadata(&[("id", text(""))]))
    )])),
    Some("")
)]
#[case::a_numeric_id_vanishes(
    Some(metadata(&[(
        "model_info",
        Json::Object(metadata(&[("id", Json::Number(Number::from(7)))]))
    )])),
    None
)]
#[case::model_info_is_not_an_object(Some(metadata(&[("model_info", text("x"))])), None)]
fn base_stream_state_reads_only_a_string_model_id(
    #[case] given: Option<IndexMap<String, Json>>,
    #[case] expected: Option<&str>,
) {
    let headers = IndexMap::new();
    let state = base_stream_state(&BaseStreamStateInput {
        model: "gpt-4o",
        custom_llm_provider: None,
        litellm_metadata: given.as_ref(),
        request_data: None,
        call_type: None,
        logging: snapshot(IndexMap::new()),
        headers: &headers,
        api_base: &NoApiBase,
        times: times(),
        max_streaming_duration_seconds: None,
    });
    assert_eq!(state.hidden_params.model_id.as_deref(), expected);
}

#[rstest]
#[case::the_param_wins(
    "gpt-4o",
    &[("api_base", "https://explicit.test")],
    Some("https://explicit.test")
)]
#[case::base_url_is_used_without_api_base(
    "gpt-4o",
    &[("base_url", "https://alias.test")],
    Some("https://alias.test")
)]
#[case::the_openai_default("openai/gpt-4o", &[], Some("https://api.openai.com"))]
#[case::the_gemini_default(
    "gemini/gemini-pro",
    &[],
    Some("https://generativelanguage.googleapis.com/v1beta/models/gemini-pro:generateContent")
)]
#[case::an_unknown_model_has_none("unknown-model-xyz", &[], None)]
fn base_stream_state_resolves_the_api_base(
    #[case] model: &str,
    #[case] params: &[(&str, &str)],
    #[case] expected: Option<&str>,
) {
    let shared = SharedResolution::default();
    let hooks = NoHooks;
    let resolution = resolution_context(&shared);
    let resolver = SharedResponsesApiBase {
        context: ApiBaseContext {
            model_alias_map: &shared.aliases,
            hooks: &hooks,
            resolution: &resolution,
        },
    };
    let headers = IndexMap::new();
    let litellm_params = metadata(
        &params
            .iter()
            .map(|(key, value)| (*key, text(value)))
            .collect::<Vec<_>>(),
    );
    let state = base_stream_state(&BaseStreamStateInput {
        model,
        custom_llm_provider: None,
        litellm_metadata: None,
        request_data: None,
        call_type: None,
        logging: snapshot(litellm_params),
        headers: &headers,
        api_base: &resolver,
        times: times(),
        max_streaming_duration_seconds: None,
    });
    assert_eq!(state.hidden_params.api_base.as_deref(), expected);
}

#[test]
fn base_stream_state_drops_a_failed_api_base_lookup() {
    let headers = IndexMap::new();
    let state = base_stream_state(&BaseStreamStateInput {
        model: "gpt-4o",
        custom_llm_provider: None,
        litellm_metadata: None,
        request_data: None,
        call_type: None,
        logging: snapshot(IndexMap::new()),
        headers: &headers,
        api_base: &FailingApiBase,
        times: times(),
        max_streaming_duration_seconds: None,
    });
    assert_eq!(state.hidden_params.api_base, None);
}

#[test]
fn base_stream_state_processes_the_additional_headers() {
    let mut headers = IndexMap::new();
    headers.insert("x-ratelimit-limit-tokens".to_owned(), "9".to_owned());
    headers.insert("x-custom".to_owned(), "v".to_owned());
    let state = base_stream_state(&BaseStreamStateInput {
        model: "gpt-4o",
        custom_llm_provider: None,
        litellm_metadata: None,
        request_data: None,
        call_type: None,
        logging: snapshot(IndexMap::new()),
        headers: &headers,
        api_base: &NoApiBase,
        times: times(),
        max_streaming_duration_seconds: None,
    });
    let additional = &state.hidden_params.additional_headers;
    assert_eq!(additional.get("x-ratelimit-limit-tokens"), Some(&text("9")));
    assert_eq!(
        additional.get("llm_provider-x-ratelimit-limit-tokens"),
        Some(&text("9"))
    );
    assert_eq!(additional.get("llm_provider-x-custom"), Some(&text("v")));
    assert_eq!(additional.get("x-custom"), None);
    headers.insert("x-custom".to_owned(), "changed".to_owned());
    assert_eq!(
        state
            .raw_response_headers
            .get("x-custom")
            .map(String::as_str),
        Some("v")
    );
}

#[test]
fn base_stream_state_reports_the_metadata_it_was_given() {
    let headers = IndexMap::new();
    let litellm_metadata =
        metadata(&[("model_info", Json::Object(metadata(&[("id", text("d-1"))])))]);
    let state = base_stream_state(&BaseStreamStateInput {
        model: "gpt-4o",
        custom_llm_provider: Some("azure"),
        litellm_metadata: Some(&litellm_metadata),
        request_data: None,
        call_type: Some("responses"),
        logging: snapshot(IndexMap::new()),
        headers: &headers,
        api_base: &NoApiBase,
        times: times(),
        max_streaming_duration_seconds: None,
    });
    assert_eq!(state.litellm_metadata.as_ref(), Some(&litellm_metadata));
    assert_eq!(state.hidden_params.model_id.as_deref(), Some("d-1"));
}
