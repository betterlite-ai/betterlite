use std::str::FromStr as _;
use std::time::Duration;

use betterlite_config::sections::vertex_credential_sources::VertexSettings;
use betterlite_config::sections::vertex_credential_sources::vertex_settings::AdcEnvironment;
use betterlite_core::cost::prepare::TypedResponse;
use betterlite_core::cost::recording::{CostRecorder, RecordInput, RecordResponse};
use betterlite_core::cost::usage::{BillingUsage, ResponseKind};
use betterlite_core::helpers::set_response_cost_in_hidden_params;
use betterlite_logging::fields::usage::Usage;
use betterlite_logging::logging_state::LoggingInit;
use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use indexmap::IndexMap;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use serde_json::json;
use tokio::runtime::Builder;

mod calculator;
mod catalog;
mod input;
mod providers;

use calculator::{Calculator, Observed};
use catalog::Catalog;
use input::{CallFields, Function, Input, LogCase, ResponseSpec, UsageFields};
use providers::{Conversion, Inference, Mantle, Regions};

const FAILURE_KEY: &str = "response_cost_failure_debug_information";
const DIAGNOSTIC_KEY: &str = "zero_cost_diagnostic";

fn failure(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input.function {
            Function::Record => record_calls(input),
            Function::SetResponseCost => set_cost(input),
        }
    }
}

fn record_calls(input: &Input) -> HelperResult {
    let runtime = Builder::new_current_thread()
        .build()
        .map_err(|error| failure(error.to_string()))?;
    runtime.block_on(record_logs(input))
}

async fn record_logs(input: &Input) -> HelperResult {
    let catalog = Catalog::new(&input.model_cost);
    let regions = Regions::new();
    let vertex = VertexSettings {
        project: None,
        location: input.vertex_location.clone(),
        credentials: None,
        adc: AdcEnvironment::default(),
        default_video_duration_seconds: 8,
    };
    let services = Services {
        catalog,
        regions,
        vertex,
        ptu_cost_attribution: input.ptu_cost_attribution,
    };
    let mut logs = Vec::new();
    for case in &input.logs {
        logs.push(record_case(&services, case).await?);
    }
    Ok(json!({ "logs": logs }))
}

struct Services {
    catalog: Catalog,
    regions: Regions,
    vertex: VertexSettings,
    ptu_cost_attribution: bool,
}

async fn record_case(services: &Services, case: &LogCase) -> Result<Json, Box<HelperError>> {
    let mut log = log_state(case)?;
    let mut calls = Vec::new();
    for call in &case.calls {
        let calculator = Calculator::new(&call.calculator);
        let recorder = CostRecorder {
            calculator: &calculator,
            lookup: &services.catalog,
            inference: &Inference,
            conversion: &Conversion,
            regions: &services.regions,
            mantle: &Mantle,
            vertex: &services.vertex,
            ptu_cost_attribution: services.ptu_cost_attribution,
        };
        let amount = record_response(&recorder, &mut log, call).await;
        calls.push(json!({
            "amount": amount.map(|cost| cost.to_f64()),
            "failure": log.model_call_details.get(FAILURE_KEY).cloned(),
            "diagnostic": log.model_call_details.get(DIAGNOSTIC_KEY).cloned(),
            "warned": log.cost.zero_cost_warned,
            "observed": observed_json(calculator.observe()),
        }));
    }
    Ok(json!({ "calls": calls }))
}

struct TypedCall<'call> {
    model: &'call str,
    usage: &'call UsageFields,
    hidden_params: &'call IndexMap<String, Json>,
    background: bool,
}

async fn record_response(
    recorder: &CostRecorder<'_, Inference>,
    log: &mut LoggingState,
    call: &CallFields,
) -> Option<Decimal> {
    let names = Names {
        cache_hit: call.cache_hit,
        litellm_model_name: call.litellm_model_name.as_deref(),
        router_model_id: call.router_model_id.as_deref(),
    };
    match &call.response {
        ResponseSpec::Typed {
            model,
            usage,
            hidden_params,
            background,
        } => {
            let typed = TypedCall {
                model: model.as_str(),
                usage,
                hidden_params,
                background: *background,
            };
            record_typed(recorder, log, typed, &names).await
        }
        ResponseSpec::Json { body } => {
            recorder
                .record(log, names.input(RecordResponse::Json(body)))
                .await
        }
        ResponseSpec::GenerateContent { body } => {
            recorder
                .record(log, names.input(RecordResponse::GenerateContent(body)))
                .await
        }
        ResponseSpec::Absent => {
            recorder
                .record(log, names.input(RecordResponse::Absent))
                .await
        }
    }
}

async fn record_typed(
    recorder: &CostRecorder<'_, Inference>,
    log: &mut LoggingState,
    call: TypedCall<'_>,
    names: &Names<'_>,
) -> Option<Decimal> {
    let built = call.usage.to_usage();
    let typed = TypedResponse {
        kind: ResponseKind::Chat,
        model: Some(call.model),
        usage: BillingUsage::Chat(&built),
        service_tier: None,
        hidden_params: Some(call.hidden_params),
    };
    let response = RecordResponse::Typed {
        response: typed,
        background: call.background,
    };
    recorder.record(log, names.input(response)).await
}

struct Names<'names> {
    cache_hit: Option<bool>,
    litellm_model_name: Option<&'names str>,
    router_model_id: Option<&'names str>,
}

impl<'names> Names<'names> {
    fn input<'response>(&self, response: RecordResponse<'response>) -> RecordInput<'response>
    where
        'names: 'response,
    {
        RecordInput {
            response,
            cache_hit: self.cache_hit,
            litellm_model_name: self.litellm_model_name,
            router_model_id: self.router_model_id,
        }
    }
}

fn observed_json(observed: Vec<Observed>) -> Vec<Json> {
    observed.iter().map(Observed::json).collect()
}

fn log_state(case: &LogCase) -> Result<LoggingState, Box<HelperError>> {
    let init = json!({
        "model": case.log.model,
        "call_type": case.log.call_type,
        "start_time": "2026-01-01T00:00:00+00:00",
        "litellm_call_id": "cost-recording-case",
        "function_id": "cost-recording-case",
        "litellm_params": case.log.litellm_params,
    });
    let text = serde_json::to_string(&init).map_err(|error| failure(error.to_string()))?;
    let parsed =
        serde_json::from_str::<LoggingInit>(&text).map_err(|error| failure(error.to_string()))?;
    let redactor =
        Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(|error| failure(error.to_string()))?;
    let mut state = LoggingState::new(
        parsed,
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        &redactor,
        || String::from("cost-recording"),
        Duration::default(),
    )
    .map_err(|error| failure(error.to_string()))?;
    state.optional_params = case.log.optional_params.clone();
    for (name, value) in &case.log.details {
        state.model_call_details.insert(name.clone(), value.clone());
    }
    Ok(state)
}

fn set_cost(input: &Input) -> HelperResult {
    let mut hidden: IndexMap<String, Json> = input.hidden_params.clone().unwrap_or_default();
    let cost = match &input.cost {
        None | Some(Json::Null) => None,
        Some(Json::Number(number)) => Some(
            Decimal::from_str(&number.to_string()).map_err(|error| failure(error.to_string()))?,
        ),
        Some(Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            return Err(failure("the cost must be a number".to_owned()));
        }
    };
    set_response_cost_in_hidden_params(&mut hidden, cost);
    Ok(json!({ "hidden_params": Json::Object(hidden) }))
}

impl UsageFields {
    fn to_usage(&self) -> Usage {
        Usage {
            prompt_tokens: self.prompt_tokens,
            completion_tokens: self.completion_tokens,
            total_tokens: self
                .total_tokens
                .unwrap_or(self.prompt_tokens.saturating_add(self.completion_tokens)),
            prompt_tokens_details: None,
            completion_tokens_details: None,
            server_tool_use: None,
            cost: None,
            extras: IndexMap::new(),
        }
    }
}
