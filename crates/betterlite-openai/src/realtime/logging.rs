use betterlite_logging::logging_pre_post::args::{AdditionalArgs, RawBody};
use betterlite_logging::logging_pre_post::context::LogContext;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_logging::logging_pre_post::pre_call::{PreCall, pre_call};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::openai_base_config::Headers;
use crate::realtime::url::QueryParams;

// The pre_call LiteLLM's OpenAI realtime handler makes: no input, the connect URL, the headers and the query mapping.
pub struct RealtimeLogRecord<'record> {
    pub api_base: &'record str,
    pub headers: &'record Headers,
    pub query_params: &'record QueryParams,
}

pub trait RealtimeLogger {
    fn pre_call(&mut self, record: RealtimeLogRecord<'_>) -> Result<(), PrePostError>;
}

impl RealtimeLogger for LogContext<'_, '_> {
    fn pre_call(&mut self, record: RealtimeLogRecord<'_>) -> Result<(), PrePostError> {
        let headers: IndexMap<String, Json> = record
            .headers
            .iter()
            .map(|(name, value)| (name.clone(), Json::Text(value.clone())))
            .collect();
        let query_params: IndexMap<String, Json> = record
            .query_params
            .iter()
            .map(|(name, value)| {
                let shown = value
                    .as_ref()
                    .map_or(Json::Null, |text| Json::Text(text.clone()));
                (name.clone(), shown)
            })
            .collect();
        pre_call(
            self.state,
            &PreCall {
                input: Json::Null,
                model: None,
                additional_args: AdditionalArgs {
                    api_base: Some(record.api_base.to_owned()),
                    headers: Some(headers),
                    complete_input_dict: Some(RawBody::Object(IndexMap::from([(
                        String::from("query_params"),
                        Json::Object(query_params),
                    )]))),
                    request_str: None,
                    openai_sdk: None,
                    passthrough: IndexMap::new(),
                },
            },
            self.env,
        )
        .map(drop)
    }
}
