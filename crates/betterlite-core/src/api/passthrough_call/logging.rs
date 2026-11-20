use std::convert::Infallible;

use async_trait::async_trait;
use betterlite_config::sections::settings::JsonValue;
use betterlite_logging::logging_pre_post::args::{AdditionalArgs, RawBody};
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_pre_post::pre_call::{PreCall, pre_call};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_state::updates::{CallUpdate, update_environment_variables};
use betterlite_logging::python_json::dumps;
use betterlite_models::decisions::Json;
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_openai::passthrough::transformation::PassthroughStreamCollector;
use betterlite_openai::passthrough::transformation::types::{
    PassthroughError, PassthroughLogging, PassthroughTarget,
};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::Utc;
use indexmap::IndexMap;

use super::config::ConfigError;
use crate::passthrough_stream::logging::SpendLogging;
use crate::passthrough_stream::spend_collection::BoxedCollector;

// The pass-through call keeps no spend log of its own; the relay still needs a sink for its collector
pub struct RelayLogging;

#[async_trait]
impl SpendLogging<()> for RelayLogging {
    type FlushError = Infallible;

    fn spend_target(&self) -> PassthroughTarget<'_> {
        PassthroughTarget {
            model: "",
            custom_llm_provider: "",
            endpoint: "",
        }
    }

    async fn async_flush_collected_chunks(
        &self,
        _collector: BoxedCollector<()>,
    ) -> Result<(), Infallible> {
        Ok(())
    }

    fn flush_collected_chunks(&self, _collector: BoxedCollector<()>) -> Result<(), Infallible> {
        Ok(())
    }
}

pub(crate) struct NoSpendCollector;

impl PassthroughStreamCollector for NoSpendCollector {
    type Logged = ();

    fn add(&mut self, _chunk: &[u8]) {}

    fn build_logged_response(
        &self,
        _logging_obj: &mut dyn PassthroughLogging,
    ) -> Result<Option<Self::Logged>, PassthroughError> {
        Ok(None)
    }
}

// The production sink: LiteLLM's debug logging writes the record and the client never sees it
struct TracingSink;

impl LogSink for TracingSink {
    fn emit(&self, record: LogRecord) {
        tracing::debug!(
            level = ?record.level,
            fields = ?record.fields,
            "{}",
            record.message
        );
    }
}

pub(crate) fn json(value: &JsonValue) -> Json {
    match value {
        JsonValue::Null => Json::Null,
        JsonValue::Bool(flag) => Json::Bool(*flag),
        JsonValue::Number(number) => Json::Number(number.clone()),
        JsonValue::String(text) => Json::Text(text.clone()),
        JsonValue::Array(items) => Json::List(items.iter().map(json).collect()),
        JsonValue::Object(fields) => Json::Object(json_object(fields)),
    }
}

pub(crate) fn json_object(fields: &Object) -> IndexMap<String, Json> {
    fields
        .iter()
        .map(|(name, value)| (name.clone(), json(value)))
        .collect()
}

// llm_passthrough_route's update_environment_variables call, with the request data it records
pub(crate) struct Environment<'call> {
    pub model: &'call str,
    pub params: &'call IndexMap<String, Json>,
    pub endpoint: &'call str,
    pub provider: &'call str,
    pub request_data: Option<&'call Json>,
}

pub(crate) fn record_environment(
    state: &mut LoggingState,
    environment: &Environment<'_>,
) -> Result<(), ConfigError> {
    let mut additional = IndexMap::new();
    additional.insert(
        "endpoint".to_owned(),
        Json::Text(environment.endpoint.to_owned()),
    );
    additional.insert(
        "custom_llm_provider".to_owned(),
        Json::Text(environment.provider.to_owned()),
    );
    additional.insert(
        "request_data".to_owned(),
        environment.request_data.cloned().unwrap_or(Json::Null),
    );
    let update = CallUpdate {
        model: Some(environment.model.to_owned()),
        user: None,
        optional_params: IndexMap::new(),
        additional_params: additional,
    };
    update_environment_variables(state, environment.params, update)
        .map_err(|error| ConfigError::value_error(error.to_string()))
}

pub(crate) struct PreCallRequest<'call> {
    pub input: Json,
    pub api_base: &'call str,
    pub headers: &'call Headers,
}

// llm_passthrough_route's pre_call: the same data it sends, its final headers and the upstream url
pub(crate) fn record_pre_call(
    state: &mut LoggingState,
    request: &PreCallRequest<'_>,
) -> Result<(), ConfigError> {
    let mut logged_headers = IndexMap::new();
    for (name, value) in request.headers {
        logged_headers.insert(name.clone(), Json::Text(value.clone()));
    }
    let call = PreCall {
        input: request.input.clone(),
        model: None,
        additional_args: AdditionalArgs {
            api_base: Some(request.api_base.to_owned()),
            headers: Some(logged_headers),
            complete_input_dict: complete_input_dict(&request.input),
            request_str: None,
            openai_sdk: None,
            passthrough: IndexMap::new(),
        },
    };
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)
        .map_err(|error| ConfigError::value_error(error.to_string()))?;
    let sink = TracingSink;
    let environment = PrePostEnv {
        debug: DebugSettings::default(),
        globals: GlobalFlags::default(),
        decision: Decision {
            content: false,
            key_info: false,
        },
        redactor: &redactor,
        log: &sink,
        input_callbacks: &[],
        sentry: None,
        logger_fn: None,
        now: Utc::now().fixed_offset(),
    };
    let _output = pre_call(state, &call, &environment)
        .map_err(|error| ConfigError::value_error(error.to_string()))?;
    Ok(())
}

// LiteLLM hands pre_call the same object it sends, so a JSON list keeps its text instead of becoming a map
fn complete_input_dict(input: &Json) -> Option<RawBody> {
    match input {
        Json::Null => None,
        Json::Object(fields) => Some(RawBody::Object(fields.clone())),
        Json::Text(text) => Some(RawBody::Text(text.clone())),
        Json::Bool(_) | Json::List(_) | Json::Number(_) => Some(RawBody::Text(dumps(input))),
    }
}
