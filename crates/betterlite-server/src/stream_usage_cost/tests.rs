#![cfg(test)]

use std::collections::BTreeMap;
use std::str::FromStr as _;
use std::sync::Mutex;
use std::time::Duration;

use betterlite_core::cost::recording::{RecordInput, RecordResponse};
use betterlite_core::cost::usage::BillingUsage;
use betterlite_logging::fields::usage::Usage;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::cost_breakdown::CostBreakdown;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::Redactor;
use chrono::DateTime;
use indexmap::IndexMap;
use rust_decimal::Decimal;

use super::pricing::RESPONSE_COST_DEBUG;
use super::{StreamCostCalculator, StreamCostInjector, StreamCostRecorder};

const CHAT_EVENT: &str =
    r#"{"object":"chat.completion.chunk","usage":{"prompt_tokens":2,"completion_tokens":1}}"#;

const DELTA_EVENT: &str = r#"{"type":"message_delta","usage":{"input_tokens":10,"output_tokens":5,"cache_read_input_tokens":3,"cache_creation_input_tokens":2,"cache_creation":{"ephemeral_5m_input_tokens":1,"ephemeral_1h_input_tokens":1}}}"#;

#[derive(Debug, PartialEq, Eq)]
struct Observed {
    service: &'static str,
    model: String,
    service_tier: Option<String>,
    prompt_tokens: i64,
    completion_tokens: i64,
    total_tokens: i64,
    prompt_details: Option<IndexMap<String, Json>>,
    completion_details: Option<IndexMap<String, Json>>,
}

struct RecordingRecorder {
    cost: Option<Decimal>,
    debug: Option<Json>,
    breakdown: Option<CostBreakdown>,
    observed: Mutex<Vec<Observed>>,
}

impl StreamCostRecorder for RecordingRecorder {
    fn response_cost(&self, log: &mut LoggingState, input: RecordInput<'_>) -> Option<Decimal> {
        let usage = chat_usage(&input);
        observe(&self.observed, "recorder", &log.model, None, &usage);
        if let Some(debug) = &self.debug {
            log.model_call_details
                .insert(RESPONSE_COST_DEBUG.to_owned(), debug.clone());
        }
        if let Some(breakdown) = &self.breakdown {
            log.cost_breakdown = Some(breakdown.clone());
        }
        self.cost
    }
}

fn chat_usage(input: &RecordInput<'_>) -> Usage {
    let RecordResponse::Typed { response, .. } = &input.response else {
        panic!("the recorder receives a typed response");
    };
    let BillingUsage::Chat(usage) = &response.usage else {
        panic!("the recorder receives a chat usage");
    };
    (*usage).clone()
}

struct RecordingCalculator {
    cost: Option<Decimal>,
    observed: Mutex<Vec<Observed>>,
}

impl StreamCostCalculator for RecordingCalculator {
    fn completion_cost(
        &self,
        usage: &Usage,
        model_name: &str,
        service_tier: Option<&str>,
    ) -> Option<Decimal> {
        observe(
            &self.observed,
            "calculator",
            model_name,
            service_tier,
            usage,
        );
        self.cost
    }
}

fn observe(
    observed: &Mutex<Vec<Observed>>,
    service: &'static str,
    model: &str,
    service_tier: Option<&str>,
    usage: &Usage,
) {
    let mut entries = observed.lock().unwrap();
    entries.push(Observed {
        service,
        model: model.to_owned(),
        service_tier: service_tier.map(str::to_owned),
        prompt_tokens: usage.prompt_tokens,
        completion_tokens: usage.completion_tokens,
        total_tokens: usage.total_tokens,
        prompt_details: usage.prompt_tokens_details.clone(),
        completion_details: usage.completion_tokens_details.clone(),
    });
}

fn take_calls(observed: &Mutex<Vec<Observed>>) -> Vec<Observed> {
    let mut entries = observed.lock().unwrap();
    entries.split_off(0)
}

fn recorder(cost: Option<&str>) -> RecordingRecorder {
    RecordingRecorder {
        cost: cost.map(|text| Decimal::from_str(text).unwrap()),
        debug: None,
        breakdown: None,
        observed: Mutex::new(Vec::new()),
    }
}

fn calculator(cost: Option<&str>) -> RecordingCalculator {
    RecordingCalculator {
        cost: cost.map(|text| Decimal::from_str(text).unwrap()),
        observed: Mutex::new(Vec::new()),
    }
}

fn injector<'services>(
    recorder: &'services RecordingRecorder,
    calculator: &'services RecordingCalculator,
    enabled: bool,
) -> StreamCostInjector<'services> {
    StreamCostInjector {
        enabled,
        recorder,
        calculator,
    }
}

fn call_log(model: &str) -> LoggingState {
    let init = LoggingInit {
        model: model.to_owned(),
        call_type: "acompletion".to_owned(),
        start_time: DateTime::parse_from_rfc3339("2026-01-01T00:00:00+00:00").unwrap(),
        litellm_call_id: "call".to_owned(),
        function_id: "function".to_owned(),
        messages: None,
        stream: Some(true),
        litellm_trace_id: None,
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
    let mut context = CorrelationContext::default();
    let setup = CorrelationSetup {
        context: &mut context,
        settings: CorrelationSettings::default(),
    };
    LoggingState::new(
        init,
        setup,
        &Redactor::new(16).unwrap(),
        || "trace".to_owned(),
        || Duration::ZERO,
    )
    .unwrap()
}

fn json(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

fn details_object(text: &str) -> IndexMap<String, Json> {
    let Json::Object(fields) = json(text) else {
        panic!("the fixture names an object");
    };
    fields
}

fn cost_text(object: &Json) -> Option<String> {
    let Json::Object(fields) = object else {
        return None;
    };
    let Some(Json::Object(usage)) = fields.get("usage") else {
        return None;
    };
    let Some(Json::Number(number)) = usage.get("cost") else {
        return None;
    };
    Some(number.to_string())
}

fn amount(text: &str) -> Decimal {
    Decimal::from_str(text).unwrap()
}

fn breakdown(total: &str) -> CostBreakdown {
    CostBreakdown {
        service_tier: None,
        data_residency: None,
        vertex_location: None,
        input_cost: None,
        cache_read_cost: None,
        cache_creation_cost: None,
        output_cost: None,
        reasoning_cost: None,
        total_cost: Some(amount(total)),
        tool_usage_cost: None,
        guardrail_cost: None,
        additional_costs: None,
        original_cost: None,
        discount_percent: None,
        discount_amount: None,
        margin_percent: None,
        margin_fixed_amount: None,
        margin_total_amount: None,
        extra: BTreeMap::new(),
    }
}

mod stream_cost_behavior;

mod stream_usage_cost_corpus;
