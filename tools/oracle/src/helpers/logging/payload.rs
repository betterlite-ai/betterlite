use std::fmt::Display;
use std::time::Duration;

use betterlite_config::sections::logging_privacy::Settings;
use betterlite_error::normalize::Normalizer;
use betterlite_logging::fields::payload::StandardLoggingPayload;
use betterlite_logging::fields::status::LoggingStatus;
use betterlite_logging::logging_metadata::base64::Base64Truncator;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_logging::payload::compose::{
    PayloadComposition, PayloadRequest, standard_logging_payload,
};
use betterlite_logging::payload::deps::{PayloadSettings, PayloadSources, TagConfig};
use betterlite_logging::payload::redaction::redact_standard_payload;
use betterlite_models::cost_breakdown::CostBreakdown;
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::Value;

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    init: LoggingInit,
    #[serde(default)]
    model_call_details: IndexMap<String, Json>,
    response: Json,
    start_time: DateTime<FixedOffset>,
    end_time: DateTime<FixedOffset>,
    #[serde(default)]
    completion_start_time: Option<DateTime<FixedOffset>>,
    status: LoggingStatus,
    #[serde(default)]
    error_str: Option<String>,
    #[serde(default)]
    standard_built_in_tools_params: Option<Json>,
    #[serde(default)]
    response_timing_metrics: IndexMap<String, f64>,
    #[serde(default)]
    cost_breakdown: Option<Json>,
    #[serde(default)]
    settings: InputSettings,
    #[serde(default)]
    redact_standard_payload: bool,
}

#[derive(Default, Deserialize)]
struct Flag(bool);

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct InputSettings {
    turn_off_message_logging: Flag,
    request_correlation_in_logs: Flag,
    log_client_error_tracebacks: Flag,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(invalid)?;
        let normalizer = Normalizer::new().map_err(invalid)?;
        let truncator = Base64Truncator::from_env(None).map_err(invalid)?;
        let mut context = CorrelationContext::default();
        let mut state = LoggingState::new(
            input.init.clone(),
            CorrelationSetup {
                context: &mut context,
                settings: CorrelationSettings::default(),
            },
            &redactor,
            || "oracle-trace".to_owned(),
            || Duration::ZERO,
        )
        .map_err(invalid)?;
        apply(&mut state, input)?;
        match build_payload(&state, input, &redactor, &normalizer, &truncator) {
            // LiteLLM's catch-all turns any failure into no payload at all
            None => Ok(Value::Null),
            Some(payload) if input.redact_standard_payload => {
                serde_json::to_value(redact_standard_payload(&payload)).map_err(invalid)
            }
            Some(payload) => serde_json::to_value(payload).map_err(invalid),
        }
    }
}

fn build_payload(
    state: &LoggingState,
    input: &Input,
    redactor: &Redactor,
    normalizer: &Normalizer,
    truncator: &Base64Truncator,
) -> Option<StandardLoggingPayload> {
    let sources = PayloadSources::default();
    let payload_settings = PayloadSettings {
        tag_config: TagConfig::default(),
        request_correlation_in_logs: input.settings.request_correlation_in_logs.0,
        log_client_error_tracebacks: input.settings.log_client_error_tracebacks.0,
    };
    let privacy = Settings {
        turn_off_message_logging: input.settings.turn_off_message_logging.0,
        redact_user_api_key_info: false,
    };
    let composition = PayloadComposition {
        sources: &sources,
        settings: &payload_settings,
        redactor,
        normalizer,
        truncator,
        privacy,
    };
    let request = PayloadRequest {
        state,
        init_response_obj: &input.response,
        start_time: input.start_time,
        end_time: input.end_time,
        status: input.status,
        error_str: input.error_str.as_deref(),
        original_exception: None,
        standard_built_in_tools_params: input.standard_built_in_tools_params.as_ref(),
    };
    standard_logging_payload(&request, &composition).ok()
}

fn apply(state: &mut LoggingState, input: &Input) -> Result<(), Box<HelperError>> {
    state
        .model_call_details
        .clone_from(&input.model_call_details);
    state.optional_params = match state.model_call_details.get("optional_params") {
        Some(Json::Object(params)) => params.clone(),
        Some(_) | None => IndexMap::new(),
    };
    state.custom_llm_provider = match state.model_call_details.get("custom_llm_provider") {
        Some(Json::Text(provider)) => Some(provider.clone()),
        Some(_) | None => None,
    };
    state.completion_start_time = input.completion_start_time;
    state
        .response_timing_metrics
        .clone_from(&input.response_timing_metrics);
    state.cost_breakdown = match input.cost_breakdown.as_ref() {
        None => None,
        Some(value) => {
            let encoded = serde_json::to_vec(value).map_err(invalid)?;
            Some(serde_json::from_slice::<CostBreakdown>(&encoded).map_err(invalid)?)
        }
    };
    Ok(())
}

fn invalid<E>(error: E) -> Box<HelperError>
where
    E: Display,
{
    Box::new(HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
