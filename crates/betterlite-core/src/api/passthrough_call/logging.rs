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
use betterlite_models::resolution::ResolvedModel;
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_openai::passthrough::transformation::PassthroughStreamCollector;
use betterlite_openai::passthrough::transformation::types::{
    PassthroughError, PassthroughLogging, PassthroughTarget,
};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::Utc;
use indexmap::IndexMap;

use super::PassthroughCallArgs;
use super::config::ConfigError;
use super::params::CallParams;
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

// llm_passthrough_route's update_environment_variables call, with the data the request will log
pub(crate) fn record_environment(
    state: &mut LoggingState,
    args: &PassthroughCallArgs,
    resolved: &ResolvedModel,
    call: &CallParams,
) -> Result<(), ConfigError> {
    let additional = IndexMap::from([
        ("endpoint".to_owned(), Json::Text(args.endpoint.clone())),
        (
            "custom_llm_provider".to_owned(),
            Json::Text(resolved.provider.slug().to_owned()),
        ),
        (
            "request_data".to_owned(),
            request_data(args).unwrap_or(Json::Null),
        ),
    ]);
    let update = CallUpdate {
        model: Some(resolved.model.clone()),
        user: None,
        optional_params: IndexMap::new(),
        additional_params: additional,
    };
    update_environment_variables(state, &call.logged, update).map_err(|error| {
        ConfigError::ValueError {
            message: error.to_string(),
        }
    })
}

// request_data for logging and pre_call: data when it is not empty, otherwise json
pub(crate) fn request_data(args: &PassthroughCallArgs) -> Option<Json> {
    match &args.data {
        Some(data) if !data.is_empty() => Some(Json::Object(json_object(data))),
        Some(_) | None => args.json.as_ref().map(json),
    }
}

fn additional_args(input: &Json, api_base: &str, headers: &Headers) -> AdditionalArgs {
    let logged_headers = headers
        .iter()
        .map(|(name, value)| (name.clone(), Json::Text(value.clone())))
        .collect();
    AdditionalArgs {
        api_base: Some(api_base.to_owned()),
        headers: Some(logged_headers),
        complete_input_dict: complete_input_dict(input),
        request_str: None,
        openai_sdk: None,
        passthrough: IndexMap::new(),
    }
}

// llm_passthrough_route's pre_call: the same data it sends, its final headers and the upstream url
pub(crate) fn record_pre_call(
    state: &mut LoggingState,
    input: &Json,
    api_base: &str,
    headers: &Headers,
) -> Result<(), ConfigError> {
    let call = PreCall {
        input: input.clone(),
        model: None,
        additional_args: additional_args(input, api_base, headers),
    };
    let redactor =
        Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(|error| ConfigError::ValueError {
            message: error.to_string(),
        })?;
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
    pre_call(state, &call, &environment).map_err(|error| ConfigError::ValueError {
        message: error.to_string(),
    })?;
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
