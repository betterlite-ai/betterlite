#![cfg(test)]

use std::io;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_config::sections::request_headers::Settings;
use betterlite_core::api::media_relay_logging::spend::decimal_json;
use betterlite_core::cost::recording::PortFailure;
use betterlite_logging::end_user::extraction::IdSettings;
use betterlite_logging::fields::payload::StandardLoggingPayload;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_logging::logging_success::ports::StandardPayloadBuilder;
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
    ResponseLoggingInput, ResponseLoggingKwargs, create_response_logging_payload,
    end_user_from_request_body,
};
use crate::pass_through_logging::response_costing::{ResponseCostRequest, ResponseCostService};

// StandardLoggingPayload denies unknown fields, so the fixture names every key; optional ones are explicit nulls.
const PAYLOAD_TEMPLATE: &str = r#"{
    "classifier_input": null,
    "originating_request_masked": null,
    "id": "chatcmpl-1",
    "trace_id": "trace-1",
    "session_id": "session-1",
    "litellm_call_id": null,
    "call_type": "pass_through_endpoint",
    "stream": null,
    "response_cost": 2.5,
    "cost_breakdown": null,
    "autorouter_savings": null,
    "autorouter_savings_estimate": null,
    "autorouter_baseline_observation": null,
    "response_cost_failure_debug_info": null,
    "zero_cost_diagnostic": null,
    "status": "success",
    "status_fields": {"llm_api_status": "success", "guardrail_status": "not_run"},
    "custom_llm_provider": null,
    "total_tokens": 5,
    "prompt_tokens": 2,
    "completion_tokens": 3,
    "startTime": 1704067200,
    "endTime": 1704067201,
    "completionStartTime": 1704067201,
    "response_time": 1,
    "model_map_information": {},
    "model": "passthrough-model",
    "model_id": null,
    "model_group": null,
    "api_base": "https://example.invalid",
    "metadata": {
        "user_api_key_hash": null,
        "user_api_key_alias": null,
        "user_api_key_spend": null,
        "user_api_key_max_budget": null,
        "user_api_key_budget_reset_at": null,
        "user_api_key_user_spend": null,
        "user_api_key_user_max_budget": null,
        "user_api_key_team_spend": null,
        "user_api_key_team_max_budget": null,
        "user_api_key_org_id": null,
        "user_api_key_org_alias": null,
        "user_api_key_team_id": null,
        "user_api_key_project_id": null,
        "user_api_key_project_alias": null,
        "user_api_key_user_id": null,
        "user_api_key_user_email": null,
        "user_api_key_team_alias": null,
        "user_api_key_end_user_id": null,
        "user_api_key_request_route": null,
        "user_api_key_auth_metadata": null,
        "spend_logs_metadata": null,
        "requester_ip_address": null,
        "user_agent": null,
        "requester_metadata": null,
        "requester_custom_headers": null,
        "prompt_management_metadata": null,
        "mcp_tool_call_metadata": null,
        "vector_store_request_metadata": null,
        "routing_decision": null,
        "applied_guardrails": null,
        "usage_object": null,
        "cold_storage_object_key": null,
        "team_alias": null,
        "team_id": null,
        "used_client_oauth_token": null
    },
    "cache_hit": null,
    "cache_key": null,
    "saved_cache_cost": 0,
    "request_tags": [],
    "request_model_access_groups": null,
    "end_user": null,
    "requester_ip_address": null,
    "user_agent": null,
    "messages": null,
    "response": null,
    "error_str": null,
    "error_information": null,
    "model_parameters": {},
    "hidden_params": {
        "model_id": null,
        "cache_key": null,
        "api_base": null,
        "response_cost": null,
        "litellm_overhead_time_ms": null,
        "additional_headers": null,
        "batch_models": null,
        "batch_successful_requests": null,
        "batch_failed_requests": null,
        "litellm_model_name": null,
        "usage_object": null
    },
    "guardrail_information": null,
    "standard_built_in_tools_params": null
}"#;

#[test]
fn payload_sets_the_cost_model_and_the_call_log_response_cost() {
    let mut response = json_object(r#"{"id": "upstream-id", "model": "upstream-model"}"#);
    let mut call_log = logging_state();
    let cost = FakeCost::priced(Decimal::new(25, 2));
    let builder = FakeBuilder::returning();
    let kwargs = create_response_logging_payload(
        &cost,
        &builder,
        input(
            &mut response,
            &mut call_log,
            logging_kwargs(JsonObject::new(), None),
            None,
        ),
    );
    assert_eq!(kwargs.response_cost, Some(Decimal::new(25, 2)));
    assert_eq!(kwargs.model.as_deref(), Some("requested-model"));
    let stored = kwargs.standard_logging_object.expect("a stored payload");
    assert_eq!(field(&stored, "id").clone(), json_text("chatcmpl-1"));
    assert_eq!(
        call_log.model_call_details.get("response_cost"),
        Some(&decimal_json(Decimal::new(25, 2)))
    );
    assert_eq!(
        call_log.model_call_details.get("model"),
        Some(&json_text("requested-model"))
    );
    assert_eq!(call_log.model, "requested-model");
    assert_eq!(field(&response, "id").clone(), json_text("call-1"));
    assert_eq!(
        field(&response, "model").clone(),
        json_text("requested-model")
    );
    let calls = cost.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].model, "requested-model");
    assert!(calls[0].provider.is_none());
    assert!(calls[0].custom_pricing.is_none());
    assert!(calls[0].router_model_id.is_none());
}

#[test]
fn payload_uses_end_user_argument() {
    let mut response = json_object(r#"{"id": "upstream-id"}"#);
    let mut call_log = logging_state();
    let mut params = JsonObject::new();
    params.insert(
        "proxy_server_request".to_owned(),
        json_object(
            r#"{"url": "https://upstream", "method": "POST", "body": {"user": "body-user"}}"#,
        ),
    );
    let kwargs = logging_kwargs(
        params,
        Some(object(r#"{"user": "body-user", "model": "m"}"#)),
    );
    let cost = FakeCost::priced(Decimal::new(25, 2));
    let builder = FakeBuilder::returning();
    let kwargs = create_response_logging_payload(
        &cost,
        &builder,
        input(&mut response, &mut call_log, kwargs, Some("argument-user")),
    );
    let expected = Json::Object(object(r#"{"body": {"user": "argument-user"}}"#));
    assert_eq!(
        kwargs.litellm_params.get("proxy_server_request"),
        Some(&expected)
    );
    assert_eq!(
        call_log.litellm_params.get("proxy_server_request"),
        Some(&expected)
    );
}

#[rstest]
#[case(None)]
#[case(Some(""))]
fn payload_skips_the_user_without_a_usable_end_user(#[case] end_user: Option<&str>) {
    let mut response = json_object(r#"{"id": "upstream-id"}"#);
    let mut call_log = logging_state();
    let kwargs = logging_kwargs(JsonObject::new(), Some(object(r#"{"user": "body-user"}"#)));
    let cost = FakeCost::priced(Decimal::new(25, 2));
    let builder = FakeBuilder::returning();
    let kwargs = create_response_logging_payload(
        &cost,
        &builder,
        input(&mut response, &mut call_log, kwargs, end_user),
    );
    assert!(kwargs.litellm_params.get("proxy_server_request").is_none());
    assert!(
        call_log
            .litellm_params
            .get("proxy_server_request")
            .is_none()
    );
}

#[test]
fn payload_skips_the_user_without_a_request_body() {
    let mut response = json_object(r#"{"id": "upstream-id"}"#);
    let mut call_log = logging_state();
    let cost = FakeCost::priced(Decimal::new(25, 2));
    let builder = FakeBuilder::returning();
    let kwargs = create_response_logging_payload(
        &cost,
        &builder,
        input(
            &mut response,
            &mut call_log,
            logging_kwargs(JsonObject::new(), None),
            Some("argument-user"),
        ),
    );
    assert!(kwargs.litellm_params.get("proxy_server_request").is_none());
    assert!(
        call_log
            .litellm_params
            .get("proxy_server_request")
            .is_none()
    );
}

#[test]
fn payload_skips_the_user_for_an_empty_request_body() {
    let mut response = json_object(r#"{"id": "upstream-id"}"#);
    let mut call_log = logging_state();
    let kwargs = logging_kwargs(JsonObject::new(), Some(JsonObject::new()));
    let cost = FakeCost::priced(Decimal::new(25, 2));
    let builder = FakeBuilder::returning();
    let kwargs = create_response_logging_payload(
        &cost,
        &builder,
        input(&mut response, &mut call_log, kwargs, Some("argument-user")),
    );
    assert!(kwargs.litellm_params.get("proxy_server_request").is_none());
    assert!(
        call_log
            .litellm_params
            .get("proxy_server_request")
            .is_none()
    );
}

#[test]
fn payload_stops_after_a_cost_failure() {
    let (log, _guard) = capture_logs(tracing::Level::ERROR);
    let mut response = json_object(r#"{"id": "upstream-id", "model": "upstream-model"}"#);
    let mut call_log = logging_state();
    let kwargs = logging_kwargs(JsonObject::new(), Some(object(r#"{"user": "body-user"}"#)));
    let cost = FakeCost::unpriced();
    let builder = FakeBuilder::returning();
    let kwargs = create_response_logging_payload(
        &cost,
        &builder,
        input(&mut response, &mut call_log, kwargs, Some("argument-user")),
    );
    assert_eq!(kwargs.response_cost, None);
    assert_eq!(kwargs.model, None);
    assert!(kwargs.standard_logging_object.is_none());
    assert!(kwargs.litellm_params.get("proxy_server_request").is_none());
    assert!(builder.calls().is_empty());
    assert!(call_log.model_call_details.get("response_cost").is_none());
    assert_eq!(field(&response, "id").clone(), json_text("upstream-id"));
    assert_eq!(
        field(&response, "model").clone(),
        json_text("upstream-model")
    );
    assert_eq!(
        call_log.model_call_details.get("model"),
        Some(&json_text(""))
    );
    let output = log.contents();
    assert!(output.contains(
        "Error creating LLM passthrough response logging payload: the response cost could not be computed: no price for this model"
    ));
}

#[test]
fn payload_keeps_finished_steps_when_the_standard_object_fails() {
    let (log, _guard) = capture_logs(tracing::Level::ERROR);
    let mut response = json_object(r#"{"id": "upstream-id", "model": "upstream-model"}"#);
    let mut call_log = logging_state();
    let kwargs = logging_kwargs(JsonObject::new(), Some(object(r#"{"user": "body-user"}"#)));
    let cost = FakeCost::priced(Decimal::new(25, 2));
    let builder = FakeBuilder::blank();
    let kwargs = create_response_logging_payload(
        &cost,
        &builder,
        input(&mut response, &mut call_log, kwargs, Some("argument-user")),
    );
    assert_eq!(kwargs.response_cost, Some(Decimal::new(25, 2)));
    assert_eq!(kwargs.model.as_deref(), Some("requested-model"));
    assert!(kwargs.standard_logging_object.is_none());
    assert_eq!(
        kwargs.litellm_params.get("proxy_server_request"),
        Some(&Json::Object(object(
            r#"{"body": {"user": "argument-user"}}"#
        )))
    );
    assert_eq!(field(&response, "id").clone(), json_text("upstream-id"));
    assert_eq!(
        field(&response, "model").clone(),
        json_text("upstream-model")
    );
    assert_eq!(
        call_log.model_call_details.get("model"),
        Some(&json_text(""))
    );
    assert_eq!(call_log.model, "requested-model");
    let calls = builder.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].model, "requested-model");
    assert_eq!(calls[0].start_time, timestamp("2024-01-01T00:00:00+00:00"));
    assert_eq!(calls[0].end_time, timestamp("2024-01-01T00:01:00+00:00"));
    let output = log.contents();
    assert!(output.contains(
        "Error creating LLM passthrough response logging payload: the standard logging object could not be built"
    ));
}

#[test]
fn payload_keeps_the_object_when_the_response_is_not_an_object() {
    let (log, _guard) = capture_logs(tracing::Level::ERROR);
    let mut response = json_text("not an object");
    let mut call_log = logging_state();
    let cost = FakeCost::priced(Decimal::new(25, 2));
    let builder = FakeBuilder::returning();
    let kwargs = create_response_logging_payload(
        &cost,
        &builder,
        input(
            &mut response,
            &mut call_log,
            logging_kwargs(JsonObject::new(), None),
            None,
        ),
    );
    assert_eq!(kwargs.response_cost, Some(Decimal::new(25, 2)));
    assert!(kwargs.standard_logging_object.is_some());
    assert_eq!(response, json_text("not an object"));
    assert_eq!(
        call_log.model_call_details.get("model"),
        Some(&json_text(""))
    );
    let output = log.contents();
    assert!(output.contains(
        "Error creating LLM passthrough response logging payload: the response body is not a JSON object"
    ));
}

#[test]
fn payload_logs_the_standard_object_at_debug() {
    let (log, _guard) = capture_logs(tracing::Level::DEBUG);
    let mut response = json_object(r#"{"id": "upstream-id"}"#);
    let mut call_log = logging_state();
    let cost = FakeCost::priced(Decimal::new(25, 2));
    let builder = FakeBuilder::returning();
    create_response_logging_payload(
        &cost,
        &builder,
        input(
            &mut response,
            &mut call_log,
            logging_kwargs(JsonObject::new(), None),
            None,
        ),
    );
    let output = log.contents();
    assert!(output.contains("standard_logging_object= {"));
    assert!(output.contains("\n  \"id\": \"chatcmpl-1\""));
}

#[test]
fn payload_builds_the_standard_object_from_the_updated_state() {
    let mut response = json_object(r#"{"id": "upstream-id"}"#);
    let mut call_log = logging_state();
    let kwargs = logging_kwargs(JsonObject::new(), Some(object(r#"{"user": "body-user"}"#)));
    let cost = FakeCost::priced(Decimal::new(25, 2));
    let builder = FakeBuilder::returning();
    create_response_logging_payload(
        &cost,
        &builder,
        input(&mut response, &mut call_log, kwargs, Some("argument-user")),
    );
    let calls = builder.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].model, "requested-model");
    assert_eq!(calls[0].response, json_object(r#"{"id": "upstream-id"}"#));
    assert_eq!(
        calls[0].litellm_params.get("proxy_server_request"),
        Some(&Json::Object(object(
            r#"{"body": {"user": "argument-user"}}"#
        )))
    );
}

#[test]
fn payload_end_user_from_request_body_reads_the_body() {
    let headers = Settings::default();
    let settings = IdSettings {
        request_headers: &headers,
        validate_end_user_id_in_db: false,
    };
    let body = object(r#"{"user": "body-user"}"#);
    assert_eq!(
        end_user_from_request_body(Some(&body), &settings),
        Some("body-user".to_owned())
    );
    assert_eq!(
        end_user_from_request_body(Some(&object(r#"{"model": "m"}"#)), &settings),
        None
    );
    assert_eq!(
        end_user_from_request_body(Some(&object(r#"{"user": ""}"#)), &settings),
        None
    );
    assert_eq!(
        end_user_from_request_body(Some(&JsonObject::new()), &settings),
        None
    );
    assert_eq!(end_user_from_request_body(None, &settings), None);
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RecordedCost {
    model: String,
    provider: Option<String>,
    custom_pricing: Option<bool>,
    router_model_id: Option<String>,
}

struct FakeCost {
    price: Option<Decimal>,
    calls: Mutex<Vec<RecordedCost>>,
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

    fn calls(&self) -> Vec<RecordedCost> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl ResponseCostService for FakeCost {
    fn completion_cost(&self, request: &ResponseCostRequest<'_>) -> Result<Decimal, PortFailure> {
        let recorded = RecordedCost {
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

#[derive(Clone, Debug, PartialEq, Eq)]
struct BuildCall {
    model: String,
    litellm_params: JsonObject,
    response: Json,
    start_time: DateTime<FixedOffset>,
    end_time: DateTime<FixedOffset>,
}

struct FakeBuilder {
    payload: Option<StandardLoggingPayload>,
    calls: Mutex<Vec<BuildCall>>,
}

impl FakeBuilder {
    fn returning() -> Self {
        Self {
            payload: Some(payload_fixture()),
            calls: Mutex::new(Vec::new()),
        }
    }

    fn blank() -> Self {
        Self {
            payload: None,
            calls: Mutex::new(Vec::new()),
        }
    }

    fn calls(&self) -> Vec<BuildCall> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl StandardPayloadBuilder for FakeBuilder {
    fn build(
        &self,
        state: &LoggingState,
        response: &Json,
        start_time: DateTime<FixedOffset>,
        end_time: DateTime<FixedOffset>,
    ) -> Option<StandardLoggingPayload> {
        let recorded = BuildCall {
            model: state.model.clone(),
            litellm_params: state.litellm_params.clone(),
            response: response.clone(),
            start_time,
            end_time,
        };
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(recorded);
        self.payload.clone()
    }
}

fn payload_fixture() -> StandardLoggingPayload {
    serde_json::from_str(PAYLOAD_TEMPLATE).expect("the payload fixture parses")
}

fn input<'input>(
    response: &'input mut Json,
    call_log: &'input mut LoggingState,
    kwargs: ResponseLoggingKwargs,
    end_user: Option<&'input str>,
) -> ResponseLoggingInput<'input> {
    ResponseLoggingInput {
        response,
        model: "requested-model",
        kwargs,
        end_user,
        start_time: timestamp("2024-01-01T00:00:00+00:00"),
        end_time: timestamp("2024-01-01T00:01:00+00:00"),
        call_log,
    }
}

fn logging_kwargs(
    litellm_params: JsonObject,
    passthrough_request_body: Option<JsonObject>,
) -> ResponseLoggingKwargs {
    ResponseLoggingKwargs {
        response_cost: None,
        model: None,
        litellm_params,
        standard_logging_object: None,
        passthrough_request_body,
    }
}

fn object(text: &str) -> JsonObject {
    serde_json::from_str(text).expect("a JSON object")
}

fn json_object(text: &str) -> Json {
    Json::Object(object(text))
}

fn json_text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn field<'json>(value: &'json Json, name: &str) -> &'json Json {
    let Json::Object(fields) = value else {
        panic!("the value is an object");
    };
    fields.get(name).expect("the object carries the key")
}

fn timestamp(text: &str) -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339(text).expect("a valid timestamp")
}

fn logging_state() -> LoggingState {
    let redactor = Redactor::new(16).expect("a valid redactor");
    let mut context = CorrelationContext::default();
    let setup = CorrelationSetup {
        context: &mut context,
        settings: CorrelationSettings::default(),
    };
    let init = LoggingInit {
        model: String::new(),
        call_type: "pass_through_endpoint".to_owned(),
        start_time: timestamp("2024-01-01T00:00:00+00:00"),
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
    };
    LoggingState::new(
        init,
        setup,
        &redactor,
        || "generated-trace".to_owned(),
        || Duration::ZERO,
    )
    .expect("a valid logging state")
}

#[derive(Clone, Default)]
struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl LogBuffer {
    fn contents(&self) -> String {
        let bytes = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        String::from_utf8(bytes.clone()).expect("the captured log lines are UTF-8")
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
