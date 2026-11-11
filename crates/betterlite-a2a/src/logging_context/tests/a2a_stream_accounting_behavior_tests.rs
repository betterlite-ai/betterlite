#![cfg(test)]

use std::time::Duration;

use betterlite_core::agents::json::AgentJson;
use betterlite_core::agents::provider::a2a::{IntegerId, JsonRpcId};
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use rstest::rstest;
use serde_json::Number;

use crate::agent_card::{AgentCard, WireValue};
use crate::logging_context::{
    A2A_COST_PARAM_KEYS, LoggingContextError, LoggingDeps, StreamingLoggingRequest,
    a2a_cost_params, build_streaming_logging_obj, get_a2a_model_info, json_rpc_id_text,
    set_agent_id_on_logging_obj, set_litellm_params, set_usage_on_logging_obj, usage_json,
};

fn fixed_time(text: &str) -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339(text).expect("the fixture is RFC 3339")
}

fn redactor() -> Redactor {
    Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the default redactor compiles")
}

fn state(details: &[(&str, Json)]) -> LoggingState {
    let mut context = CorrelationContext::default();
    let mut logging = LoggingState::new(
        LoggingInit {
            model: "a2a_agent/agent".to_owned(),
            call_type: "asend_message_streaming".to_owned(),
            start_time: fixed_time("2026-01-01T00:00:00+00:00"),
            litellm_call_id: "call-1".to_owned(),
            function_id: "function-1".to_owned(),
            messages: None,
            stream: Some(false),
            litellm_trace_id: Some("trace-1".to_owned()),
            kwargs: IndexMap::new(),
            litellm_params: IndexMap::new(),
            callback_dynamic_params: IndexMap::new(),
            trusted_callback_vars: Vec::new(),
            standard_built_in_tools_params: IndexMap::new(),
            dynamic_callbacks: DynamicCallbacks::default(),
            applied_guardrails: None,
            request_logging: RequestLoggingFlags::default(),
            callback_policy: CallbackPolicy::default(),
        },
        CorrelationSetup {
            context: &mut context,
            settings: CorrelationSettings::default(),
        },
        &redactor(),
        || "generated-trace".to_owned(),
        || Duration::ZERO,
    )
    .expect("the fixture builds");
    let mut call_details = IndexMap::new();
    for (name, value) in details {
        call_details.insert((*name).to_owned(), value.clone());
    }
    logging.model_call_details = call_details;
    logging
}

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn number(value: u64) -> Json {
    Json::Number(Number::from(value))
}

fn object(pairs: &[(&str, Json)]) -> Json {
    Json::Object(params(pairs))
}

fn params(pairs: &[(&str, Json)]) -> IndexMap<String, Json> {
    let mut values = IndexMap::new();
    for (name, value) in pairs {
        values.insert((*name).to_owned(), value.clone());
    }
    values
}

fn details_of(logging: &LoggingState, key: &str) -> Option<Json> {
    logging.model_call_details.get(key).cloned()
}

fn numeric_id(value: u64) -> JsonRpcId {
    JsonRpcId::Integer(
        IntegerId::try_from(Number::from(value)).expect("the fixture id is an integer"),
    )
}

fn build_logging(
    request: &StreamingLoggingRequest<'_>,
    generated_trace_id: &str,
) -> Result<LoggingState, LoggingContextError> {
    let mut context = CorrelationContext::default();
    let redactor = redactor();
    build_streaming_logging_obj(
        request,
        LoggingDeps {
            start_time: fixed_time("2026-01-01T00:00:00+00:00"),
            correlation: CorrelationSetup {
                context: &mut context,
                settings: CorrelationSettings::default(),
            },
            redactor: &redactor,
            generated_trace_id,
            clock: || Duration::ZERO,
        },
    )
}

#[test]
fn usage_gets_the_prompt_completion_and_total_counts() {
    let mut logging = state(&[("metadata", object(&[("user_id", text("u-1"))]))]);

    set_usage_on_logging_obj(Some(&mut logging), 3, 4).expect("the counts fit");

    assert_eq!(
        details_of(&logging, "usage"),
        Some(object(&[
            ("prompt_tokens", number(3)),
            ("completion_tokens", number(4)),
            ("total_tokens", number(7)),
        ]))
    );
    assert_eq!(
        details_of(&logging, "metadata"),
        Some(object(&[("user_id", text("u-1"))]))
    );
}

#[test]
fn usage_replaces_an_existing_usage_object() {
    let mut logging = state(&[(
        "usage",
        object(&[("prompt_tokens", number(9)), ("total_tokens", number(9))]),
    )]);

    set_usage_on_logging_obj(Some(&mut logging), 2, 0).expect("the counts fit");

    assert_eq!(
        details_of(&logging, "usage"),
        Some(object(&[
            ("prompt_tokens", number(2)),
            ("completion_tokens", number(0)),
            ("total_tokens", number(2)),
        ]))
    );
}

#[test]
fn a_usage_total_past_u64_max_is_an_error_and_changes_nothing() {
    let mut logging = state(&[("model", text("a2a_agent/agent"))]);

    let outcome = set_usage_on_logging_obj(Some(&mut logging), u64::MAX, 1);

    assert!(matches!(outcome, Err(LoggingContextError::UsageOverflow)));
    assert_eq!(details_of(&logging, "usage"), None);
}

#[test]
fn usage_without_a_logging_object_is_a_no_op() {
    set_usage_on_logging_obj(None, 3, 4).expect("there is nothing to write");
}

#[test]
fn agent_id_is_written_when_present() {
    let mut logging = state(&[("litellm_params", object(&[("cost_per_query", number(1))]))]);

    set_agent_id_on_logging_obj(Some(&mut logging), Some("agent-9"));

    assert_eq!(details_of(&logging, "agent_id"), Some(text("agent-9")));
    assert_eq!(
        details_of(&logging, "litellm_params"),
        Some(object(&[("cost_per_query", number(1))]))
    );
}

#[test]
fn a_missing_agent_id_keeps_an_earlier_one() {
    let mut logging = state(&[("agent_id", text("keep-me"))]);

    set_agent_id_on_logging_obj(Some(&mut logging), None);

    assert_eq!(details_of(&logging, "agent_id"), Some(text("keep-me")));
}

#[test]
fn an_empty_agent_id_is_written_as_given() {
    let mut logging = state(&[]);

    set_agent_id_on_logging_obj(Some(&mut logging), Some(""));

    assert_eq!(details_of(&logging, "agent_id"), Some(text("")));
}

#[test]
fn agent_id_without_a_logging_object_is_a_no_op() {
    set_agent_id_on_logging_obj(None, Some("agent-9"));
}

#[rstest]
#[case::only_pricing_keys(
    Some(params(&[
        ("cost_per_query", number(5)),
        ("input_cost_per_token", text("0.01")),
        ("output_cost_per_token", number(2)),
        ("api_key", text("sk-secret")),
        ("metadata", object(&[("user_api_key", text("hash-1"))])),
        ("is_public", Json::Bool(true)),
    ])),
    params(&[
        ("cost_per_query", number(5)),
        ("input_cost_per_token", text("0.01")),
        ("output_cost_per_token", number(2)),
    ])
)]
#[case::null_prices_are_skipped(
    Some(params(&[
        ("cost_per_query", Json::Null),
        ("input_cost_per_token", number(1)),
        ("output_cost_per_token", Json::Null),
    ])),
    params(&[("input_cost_per_token", number(1))])
)]
#[case::an_absent_map_has_no_prices(None, params(&[]))]
#[case::an_empty_map_has_no_prices(Some(params(&[])), params(&[]))]
fn cost_params_keep_only_the_pricing_keys(
    #[case] input: Option<IndexMap<String, Json>>,
    #[case] expected: IndexMap<String, Json>,
) {
    assert_eq!(a2a_cost_params(input.as_ref()), expected);
}

#[test]
fn the_pricing_key_constant_lists_the_three_prices() {
    assert_eq!(
        A2A_COST_PARAM_KEYS,
        [
            "cost_per_query",
            "input_cost_per_token",
            "output_cost_per_token"
        ]
    );
}

#[test]
fn litellm_params_merge_into_an_existing_map_and_keep_proxy_metadata() {
    let mut logging = state(&[(
        "litellm_params",
        object(&[
            ("proxy_server_request", object(&[("url", text("/v1/a2a"))])),
            ("metadata", object(&[("user_id", text("u-1"))])),
            ("user_api_key", text("hash-1")),
        ]),
    )]);

    set_litellm_params(
        Some(&mut logging),
        &params(&[
            ("cost_per_query", number(5)),
            ("api_key", text("sk-secret")),
        ]),
    )
    .expect("the parameters merge");

    assert_eq!(
        details_of(&logging, "litellm_params"),
        Some(object(&[
            ("proxy_server_request", object(&[("url", text("/v1/a2a"))])),
            ("metadata", object(&[("user_id", text("u-1"))])),
            ("user_api_key", text("hash-1")),
            ("cost_per_query", number(5)),
        ]))
    );
}

#[test]
fn litellm_params_without_prices_leave_the_existing_map_alone() {
    let existing = object(&[("user_api_key", text("hash-1"))]);
    let mut logging = state(&[("litellm_params", existing.clone())]);

    set_litellm_params(
        Some(&mut logging),
        &params(&[("api_key", text("sk-secret"))]),
    )
    .expect("nothing merges");

    assert_eq!(details_of(&logging, "litellm_params"), Some(existing));
}

#[rstest]
#[case::absent(None)]
#[case::null(Some(Json::Null))]
fn litellm_params_start_a_new_map_when_missing(#[case] existing: Option<Json>) {
    let details = existing.map_or_else(Vec::new, |value| vec![("litellm_params", value)]);
    let mut logging = state(&details);

    set_litellm_params(
        Some(&mut logging),
        &params(&[("output_cost_per_token", number(2))]),
    )
    .expect("the parameters merge");

    assert_eq!(
        details_of(&logging, "litellm_params"),
        Some(object(&[("output_cost_per_token", number(2))]))
    );
}

#[test]
fn litellm_params_reject_a_non_object_existing_value() {
    let mut logging = state(&[("litellm_params", text("not-an-object"))]);

    let outcome = set_litellm_params(
        Some(&mut logging),
        &params(&[("cost_per_query", number(5))]),
    );

    assert!(matches!(
        outcome,
        Err(LoggingContextError::NotObject {
            field: "litellm_params"
        })
    ));
    assert_eq!(
        details_of(&logging, "litellm_params"),
        Some(text("not-an-object"))
    );
}

#[test]
fn litellm_params_without_a_logging_object_are_a_no_op() {
    set_litellm_params(None, &params(&[("cost_per_query", number(5))]))
        .expect("there is nothing to merge");
}

fn card_with_name(name: Option<WireValue<String>>) -> AgentCard {
    let mut card: AgentCard = serde_json::from_str("{}").expect("the empty card parses");
    card.name = name.map(Some);
    card
}

fn typed_card(name: &str) -> AgentCard {
    card_with_name(Some(WireValue::Typed(name.to_owned())))
}

fn unnamed_card() -> AgentCard {
    serde_json::from_str("{}").expect("the empty card parses")
}

#[test]
fn model_info_sets_the_agent_model_and_provider() {
    let agent_card = typed_card("card-agent");
    let mut logging = state(&[("litellm_params", object(&[("cost_per_query", number(1))]))]);

    let agent_name = get_a2a_model_info(Some(&agent_card), Some(&mut logging));

    assert_eq!(agent_name, "card-agent");
    assert_eq!(logging.model, "a2a_agent/card-agent");
    assert_eq!(logging.custom_llm_provider.as_deref(), Some("a2a_agent"));
    assert_eq!(
        details_of(&logging, "model"),
        Some(text("a2a_agent/card-agent"))
    );
    assert_eq!(
        details_of(&logging, "custom_llm_provider"),
        Some(text("a2a_agent"))
    );
    assert_eq!(
        details_of(&logging, "litellm_params"),
        Some(object(&[("cost_per_query", number(1))]))
    );
}

#[rstest]
#[case::absent_card(None)]
#[case::card_without_a_name(Some(unnamed_card()))]
fn model_info_without_a_name_names_the_agent_unknown(#[case] agent_card: Option<AgentCard>) {
    let mut logging = state(&[]);

    let agent_name = get_a2a_model_info(agent_card.as_ref(), Some(&mut logging));

    assert_eq!(agent_name, "unknown");
    assert_eq!(logging.model, "a2a_agent/unknown");
}

#[test]
fn model_info_reads_a_passthrough_string_name() {
    let agent_card = card_with_name(Some(WireValue::Passthrough(AgentJson::String(
        "passthrough-agent".to_owned(),
    ))));

    assert_eq!(
        get_a2a_model_info(Some(&agent_card), None),
        "passthrough-agent"
    );
}

#[test]
fn an_empty_card_name_is_used_as_given() {
    let agent_card = typed_card("");

    assert_eq!(get_a2a_model_info(Some(&agent_card), None), "");
}

#[test]
fn json_rpc_ids_keep_their_text_form() {
    let text_id = JsonRpcId::String("req-7".to_owned());
    let number_id = numeric_id(12);

    assert_eq!(json_rpc_id_text(&text_id), "req-7".to_owned());
    assert_eq!(json_rpc_id_text(&number_id), "12".to_owned());
}

fn full_request<'request>(
    request_id: &'request JsonRpcId,
    litellm_params: &'request IndexMap<String, Json>,
    metadata: &'request Json,
    proxy: &'request Json,
) -> StreamingLoggingRequest<'request> {
    StreamingLoggingRequest {
        request_id,
        agent_name: "card-agent",
        agent_id: Some("agent-9"),
        litellm_params: Some(litellm_params),
        metadata: Some(metadata),
        proxy_server_request: Some(proxy),
    }
}

fn pricing_params() -> IndexMap<String, Json> {
    params(&[
        ("cost_per_query", number(5)),
        ("input_cost_per_token", number(1)),
        ("api_key", text("sk-secret")),
        ("metadata", object(&[("forwarded", Json::Bool(true))])),
    ])
}

fn metadata_object() -> Json {
    object(&[("user_api_key", text("hash-1"))])
}

fn proxy_object() -> Json {
    object(&[("url", text("/v1/a2a"))])
}

#[test]
fn streaming_logging_builds_the_object_from_the_request() {
    let id = JsonRpcId::String("req-7".to_owned());
    let litellm_params = pricing_params();
    let metadata = metadata_object();
    let proxy = proxy_object();
    let request = full_request(&id, &litellm_params, &metadata, &proxy);

    let logging = build_logging(&request, "generated-trace").expect("the logging object builds");

    assert_eq!(logging.model, "a2a_agent/card-agent");
    assert_eq!(logging.custom_llm_provider.as_deref(), Some("a2a_agent"));
    assert_eq!(logging.call_type, "asend_message_streaming");
    assert_eq!(logging.stream, Some(false));
    assert_eq!(logging.litellm_call_id, "req-7");
    assert_eq!(logging.function_id, "req-7");
    assert_eq!(logging.litellm_trace_id, "generated-trace");
    assert_eq!(
        logging.messages,
        Some(vec![object(&[
            ("role", text("user")),
            ("content", text("streaming-request")),
        ])])
    );
}

#[test]
fn streaming_logging_merges_the_prices_and_the_request_context() {
    let id = JsonRpcId::String("req-7".to_owned());
    let litellm_params = pricing_params();
    let metadata = metadata_object();
    let proxy = proxy_object();
    let request = full_request(&id, &litellm_params, &metadata, &proxy);
    let expected = object(&[
        ("cost_per_query", number(5)),
        ("input_cost_per_token", number(1)),
        ("metadata", metadata_object()),
        ("proxy_server_request", proxy_object()),
    ]);

    let logging = build_logging(&request, "generated-trace").expect("the logging object builds");

    assert_eq!(Json::Object(logging.litellm_params.clone()), expected);
    assert_eq!(Json::Object(logging.optional_params.clone()), expected);
    assert_eq!(details_of(&logging, "litellm_params"), Some(expected));
    assert_eq!(details_of(&logging, "metadata"), Some(metadata_object()));
    assert_eq!(details_of(&logging, "agent_id"), Some(text("agent-9")));
    assert_eq!(
        details_of(&logging, "call_type"),
        Some(text("asend_message_streaming"))
    );
    assert_eq!(
        details_of(&logging, "model"),
        Some(text("a2a_agent/card-agent"))
    );
}

#[test]
fn streaming_logging_uses_the_integer_request_id_as_text() {
    let id = numeric_id(12);
    let request = StreamingLoggingRequest {
        request_id: &id,
        agent_name: "other-agent",
        agent_id: None,
        litellm_params: None,
        metadata: None,
        proxy_server_request: None,
    };

    let logging = build_logging(&request, "generated-trace").expect("the logging object builds");

    assert_eq!(logging.litellm_call_id, "12");
    assert_eq!(logging.function_id, "12");
    assert_eq!(logging.model, "a2a_agent/other-agent");
    assert!(logging.litellm_params.is_empty());
    assert!(logging.optional_params.is_empty());
    assert_eq!(details_of(&logging, "agent_id"), None);
    assert_eq!(
        details_of(&logging, "metadata"),
        Some(Json::Object(IndexMap::new()))
    );
}

#[test]
fn streaming_logging_keeps_an_explicit_empty_metadata_object() {
    let id = JsonRpcId::String("req-8".to_owned());
    let litellm_params = params(&[("output_cost_per_token", number(2))]);
    let empty = Json::Object(IndexMap::new());
    let request = StreamingLoggingRequest {
        request_id: &id,
        agent_name: "card-agent",
        agent_id: None,
        litellm_params: Some(&litellm_params),
        metadata: Some(&empty),
        proxy_server_request: None,
    };

    let logging = build_logging(&request, "generated-trace").expect("the logging object builds");

    assert_eq!(
        logging.litellm_params,
        params(&[
            ("output_cost_per_token", number(2)),
            ("metadata", Json::Object(IndexMap::new())),
        ])
    );
}

#[rstest]
#[case::metadata("metadata")]
#[case::proxy_server_request("proxy_server_request")]
fn streaming_logging_rejects_a_non_object_request_context(#[case] field: &'static str) {
    let id = JsonRpcId::String("req-9".to_owned());
    let wrong = text("not-an-object");
    let request = StreamingLoggingRequest {
        request_id: &id,
        agent_name: "card-agent",
        agent_id: None,
        litellm_params: None,
        metadata: (field == "metadata").then_some(&wrong),
        proxy_server_request: (field == "proxy_server_request").then_some(&wrong),
    };

    let outcome = build_logging(&request, "generated-trace");

    assert!(matches!(
        outcome,
        Err(LoggingContextError::NotObject { field: found }) if found == field
    ));
}

#[test]
fn usage_json_carries_the_three_counts() {
    assert_eq!(
        usage_json(0, 0).expect("the counts fit"),
        object(&[
            ("prompt_tokens", number(0)),
            ("completion_tokens", number(0)),
            ("total_tokens", number(0)),
        ])
    );
}
