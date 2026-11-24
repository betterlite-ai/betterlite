#![cfg(test)]

use std::io;
use std::str::from_utf8;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_core::cost::recording::PortFailure;
use betterlite_logging::logging_metadata::router::RouterModelError;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_redact::secret::Redactor;
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use rstest::rstest;
use rust_decimal::Decimal;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::MakeWriter;

use super::{
    ResponseCostError, ResponseCostRequest, ResponseCostService, compute_response_cost,
    cost_partial_stream_or_zero, cost_relevant_speed, resolve_costing_model,
};

#[test]
fn costing_speed_reads_only_a_string_speed() {
    assert_eq!(cost_relevant_speed(None), None);
    assert_eq!(cost_relevant_speed(Some(&body("{}"))), None);
    assert_eq!(
        cost_relevant_speed(Some(&body(r#"{"speed": "fast"}"#))),
        Some("fast".to_owned())
    );
    assert_eq!(
        cost_relevant_speed(Some(&body(r#"{"speed": ""}"#))),
        Some(String::new())
    );
    assert_eq!(cost_relevant_speed(Some(&body(r#"{"speed": 1}"#))), None);
    assert_eq!(cost_relevant_speed(Some(&body(r#"{"speed": null}"#))), None);
}

#[test]
fn costing_resolve_keeps_a_known_model() {
    let call_log = logging_state(&[("model", Json::Text("deployment".to_owned()))], &[]);
    assert_eq!(resolve_costing_model("given", &call_log), "given");
}

#[rstest]
#[case("")]
#[case("unknown")]
fn costing_resolve_uses_the_deployment_model(#[case] given: &str) {
    let call_log = logging_state(&[("model", Json::Text("deployment".to_owned()))], &[]);
    assert_eq!(resolve_costing_model(given, &call_log), "deployment");
}

#[test]
fn costing_resolve_uses_the_model_group_and_strips_one_prefix() {
    let call_log = logging_state(
        &[(
            "metadata",
            object(r#"{"model_group": "passthrough/passthrough/group"}"#),
        )],
        &[],
    );
    assert_eq!(
        resolve_costing_model("unknown", &call_log),
        "passthrough/group"
    );
    let plain = logging_state(&[("metadata", object(r#"{"model_group": "group"}"#))], &[]);
    assert_eq!(resolve_costing_model("", &plain), "group");
}

#[test]
fn costing_resolve_prefers_the_deployment_over_the_model_group() {
    let call_log = logging_state(
        &[
            ("model", Json::Text("deployment".to_owned())),
            ("metadata", object(r#"{"model_group": "group"}"#)),
        ],
        &[],
    );
    assert_eq!(resolve_costing_model("unknown", &call_log), "deployment");
}

#[test]
fn costing_resolve_skips_an_unusable_deployment_for_the_model_group() {
    let unknown = logging_state(
        &[
            ("model", Json::Text("unknown".to_owned())),
            ("metadata", object(r#"{"model_group": "group"}"#)),
        ],
        &[],
    );
    assert_eq!(resolve_costing_model("unknown", &unknown), "group");
    let empty = logging_state(
        &[
            ("model", Json::Text(String::new())),
            ("metadata", object(r#"{"model_group": "group"}"#)),
        ],
        &[],
    );
    assert_eq!(resolve_costing_model("", &empty), "group");
}

#[test]
fn costing_resolve_ignores_an_unknown_deployment_and_an_empty_group() {
    let call_log = logging_state(
        &[
            ("model", Json::Text("unknown".to_owned())),
            ("metadata", object(r#"{"model_group": ""}"#)),
        ],
        &[],
    );
    assert_eq!(resolve_costing_model("unknown", &call_log), "unknown");
    let no_params = logging_state(&[], &[]);
    assert_eq!(resolve_costing_model("", &no_params), "");
}

#[test]
fn costing_resolve_reads_a_null_metadata_as_empty() {
    let call_log = logging_state(
        &[
            ("metadata", Json::Null),
            ("litellm_metadata", Json::Text("text".to_owned())),
        ],
        &[],
    );
    assert_eq!(resolve_costing_model("unknown", &call_log), "unknown");
}

#[test]
fn costing_resolve_reads_the_call_log_details() {
    let mut call_log = logging_state(&[("model", Json::Text("attribute".to_owned()))], &[]);
    call_log.model_call_details.insert(
        "litellm_params".to_owned(),
        object(r#"{"model": "details-model"}"#),
    );
    assert_eq!(resolve_costing_model("unknown", &call_log), "details-model");
}

#[test]
fn costing_compute_returns_zero_on_an_exact_cache_hit() {
    let call_log = logging_state(
        &[],
        &[
            ("cache_hit", Json::Bool(true)),
            ("custom_llm_provider", Json::Text("anthropic".to_owned())),
        ],
    );
    let cost = FakeCost::priced(Decimal::new(25, 2));
    let amount = compute_response_cost(&cost, &object("{}"), "claude", &call_log).expect("a price");
    assert_eq!(amount, Decimal::ZERO);
    assert!(cost.calls().is_empty());
}

#[rstest]
#[case(Json::Bool(false))]
#[case(Json::Text("true".to_owned()))]
#[case(Json::Number(serde_json::Number::from(1_u8)))]
fn costing_compute_treats_other_cache_hit_values_as_misses(#[case] marker: Json) {
    let call_log = logging_state(&[], &[("cache_hit", marker)]);
    let cost = FakeCost::priced(Decimal::new(25, 2));
    let amount = compute_response_cost(&cost, &object("{}"), "claude", &call_log).expect("a price");
    assert_eq!(amount, Decimal::new(25, 2));
    assert_eq!(cost.calls().len(), 1);
}

#[rstest]
#[case(None, "claude", None, "claude")]
#[case(Some(""), "claude", None, "claude")]
#[case(Some("anthropic"), "claude", Some("anthropic"), "anthropic/claude")]
#[case(
    Some("anthropic"),
    "anthropic/claude",
    Some("anthropic"),
    "anthropic/claude"
)]
fn costing_compute_prefixes_the_provider_once(
    #[case] provider: Option<&str>,
    #[case] model: &str,
    #[case] recorded_provider: Option<&str>,
    #[case] recorded_model: &str,
) {
    let mut details = Vec::new();
    if let Some(configured) = provider {
        details.push(("custom_llm_provider", Json::Text(configured.to_owned())));
    }
    let call_log = logging_state(&[], &details);
    let cost = FakeCost::priced(Decimal::new(25, 2));
    let amount = compute_response_cost(&cost, &object("{}"), model, &call_log).expect("a price");
    assert_eq!(amount, Decimal::new(25, 2));
    let calls = cost.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].model, recorded_model);
    assert_eq!(calls[0].provider, recorded_provider.map(str::to_owned));
}

#[test]
fn costing_compute_passes_custom_pricing_and_the_router_id() {
    let call_log = logging_state(
        &[
            ("input_cost_per_token", Json::Bool(true)),
            ("metadata", object(r#"{"model_info": {"id": "router-1"}}"#)),
        ],
        &[],
    );
    let cost = FakeCost::priced(Decimal::new(25, 2));
    compute_response_cost(&cost, &object("{}"), "claude", &call_log).expect("a price");
    let calls = cost.calls();
    assert_eq!(calls[0].custom_pricing, Some(true));
    assert_eq!(calls[0].router_model_id.as_deref(), Some("router-1"));
}

#[test]
fn costing_compute_counts_model_info_custom_pricing() {
    let call_log = logging_state(
        &[(
            "litellm_metadata",
            object(r#"{"model_info": {"off_peak_pricing": {"peaks": []}}}"#),
        )],
        &[],
    );
    let cost = FakeCost::priced(Decimal::new(25, 2));
    compute_response_cost(&cost, &object("{}"), "claude", &call_log).expect("a price");
    assert_eq!(cost.calls()[0].custom_pricing, Some(true));
}

#[test]
fn costing_compute_counts_model_info_pricing_fields() {
    let call_log = logging_state(
        &[(
            "metadata",
            object(r#"{"model_info": {"input_cost_per_token": 0}}"#),
        )],
        &[],
    );
    let cost = FakeCost::priced(Decimal::new(25, 2));
    compute_response_cost(&cost, &object("{}"), "claude", &call_log).expect("a price");
    assert_eq!(cost.calls()[0].custom_pricing, Some(true));
}

#[test]
fn costing_compute_ignores_a_top_level_off_peak_pricing() {
    let call_log = logging_state(
        &[("off_peak_pricing", object(r#"{"input_cost_per_token": 1}"#))],
        &[],
    );
    let cost = FakeCost::priced(Decimal::new(25, 2));
    compute_response_cost(&cost, &object("{}"), "claude", &call_log).expect("a price");
    assert_eq!(cost.calls()[0].custom_pricing, Some(false));
    assert!(cost.calls()[0].router_model_id.is_none());
}

#[test]
fn costing_compute_reports_a_router_id_of_the_wrong_type() {
    let call_log = logging_state(&[("metadata", object(r#"{"model_info": {"id": 7}}"#))], &[]);
    let cost = FakeCost::priced(Decimal::new(25, 2));
    let error =
        compute_response_cost(&cost, &object("{}"), "claude", &call_log).expect_err("a typed id");
    assert!(matches!(
        error,
        ResponseCostError::Router(RouterModelError::ModelIdType)
    ));
    assert!(cost.calls().is_empty());
}

#[test]
fn costing_partial_stream_returns_the_cost() {
    let call_log = logging_state(&[], &[]);
    let cost = FakeCost::priced(Decimal::new(25, 2));
    assert_eq!(
        cost_partial_stream_or_zero(&cost, &object("{}"), "unknown", &call_log),
        Decimal::new(25, 2)
    );
}

#[test]
fn costing_partial_stream_costs_the_resolved_model() {
    let call_log = logging_state(&[("model", Json::Text("deployment".to_owned()))], &[]);
    let cost = FakeCost::priced(Decimal::new(25, 2));
    let amount = cost_partial_stream_or_zero(&cost, &object("{}"), "unknown", &call_log);
    assert_eq!(amount, Decimal::new(25, 2));
    assert_eq!(cost.calls()[0].model, "deployment");
}

#[test]
fn costing_partial_stream_warns_and_returns_zero() {
    let (log, _guard) = capture_logs(tracing::Level::WARN);
    let call_log = logging_state(&[], &[]);
    let cost = FakeCost::unpriced();
    let amount = cost_partial_stream_or_zero(&cost, &object("{}"), "unknown", &call_log);
    assert_eq!(amount, Decimal::ZERO);
    let text = log.contents();
    assert!(text.contains(
        "Anthropic passthrough: could not cost the partial usage of an interrupted stream (model=unknown): "
    ));
    assert!(text.contains("no price for this model"));
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RecordedRequest {
    model: String,
    provider: Option<String>,
    custom_pricing: Option<bool>,
    router_model_id: Option<String>,
}

struct FakeCost {
    price: Option<Decimal>,
    calls: Mutex<Vec<RecordedRequest>>,
}

impl FakeCost {
    fn priced(price: Decimal) -> Self {
        Self {
            price: Some(price),
            calls: Mutex::new(Vec::new()),
        }
    }

    fn unpriced() -> Self {
        Self {
            price: None,
            calls: Mutex::new(Vec::new()),
        }
    }

    fn calls(&self) -> Vec<RecordedRequest> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl ResponseCostService for FakeCost {
    fn completion_cost(&self, request: &ResponseCostRequest<'_>) -> Result<Decimal, PortFailure> {
        let recorded = RecordedRequest {
            model: request.model.to_owned(),
            provider: request.custom_llm_provider.map(str::to_owned),
            custom_pricing: request.custom_pricing,
            router_model_id: request.router_model_id.map(str::to_owned),
        };
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(recorded);
        self.price
            .ok_or_else(|| PortFailure::from("no price for this model"))
    }
}

fn object(text: &str) -> Json {
    serde_json::from_str(text).expect("a JSON object")
}

fn body(text: &str) -> JsonObject {
    serde_json::from_str(text).expect("a JSON object")
}

fn timestamp() -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339("2024-01-01T00:00:00+00:00").expect("a valid timestamp")
}

fn logging_state(litellm_params: &[(&str, Json)], details: &[(&str, Json)]) -> LoggingState {
    let redactor = Redactor::new(16).expect("a valid redactor");
    let mut state = LoggingState::new(
        LoggingInit {
            model: String::new(),
            call_type: "pass_through_endpoint".to_owned(),
            start_time: timestamp(),
            litellm_call_id: "call-1".to_owned(),
            function_id: "function-1".to_owned(),
            messages: None,
            stream: Some(false),
            litellm_trace_id: Some("trace-1".to_owned()),
            kwargs: IndexMap::new(),
            litellm_params: params(litellm_params),
            callback_dynamic_params: IndexMap::new(),
            trusted_callback_vars: Vec::new(),
            standard_built_in_tools_params: IndexMap::new(),
            dynamic_callbacks: DynamicCallbacks::default(),
            applied_guardrails: None,
            request_logging: RequestLoggingFlags::default(),
            callback_policy: CallbackPolicy::default(),
        },
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        &redactor,
        || "generated-trace".to_owned(),
        || Duration::ZERO,
    )
    .expect("a valid logging state");
    for (name, value) in details {
        state
            .model_call_details
            .insert((*name).to_owned(), value.clone());
    }
    state
}

fn params(entries: &[(&str, Json)]) -> IndexMap<String, Json> {
    let mut map = IndexMap::new();
    for (name, value) in entries {
        map.insert((*name).to_owned(), value.clone());
    }
    map
}

#[derive(Clone, Default)]
struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl LogBuffer {
    fn contents(&self) -> String {
        let bytes = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        from_utf8(&bytes)
            .expect("the captured log lines are UTF-8")
            .to_owned()
    }
}

impl io::Write for LogBuffer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl MakeWriter<'_> for LogBuffer {
    type Writer = Self;

    fn make_writer(&self) -> Self {
        self.clone()
    }
}

fn capture_logs(level: tracing::Level) -> (LogBuffer, DefaultGuard) {
    let buffer = LogBuffer::default();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_target(false)
        .with_max_level(level)
        .with_writer(buffer.clone())
        .finish();
    (buffer, set_default(subscriber))
}
