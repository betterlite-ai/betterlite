use betterlite_logging::logging_pre_post::args::{AdditionalArgs, RawBody};
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_logging::logging_pre_post::post_call::{PostCall, post_call};
use betterlite_logging::logging_pre_post::pre_call::{PreCall, pre_call};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::image_call::plan::LogContext;

#[derive(Clone, Copy)]
pub enum CallEvent<'event> {
    Pre { api_base: &'event str },
    Post { response: &'event Json },
}

// Construct the synchronous logging context inside this callback, never across an await.
pub type CallLogger<'logger> =
    dyn FnMut(&str, CallEvent<'_>) -> Result<(), PrePostError> + Send + 'logger;

// the handler logs the batch id as its input and the job identifier as its complete input
pub fn log_call(
    batch_id: &str,
    event: CallEvent<'_>,
    log: &mut LogContext<'_, '_>,
) -> Result<(), PrePostError> {
    let input = Json::Text(batch_id.to_owned());
    match event {
        CallEvent::Pre { api_base } => pre_call(
            log.state,
            &PreCall {
                input,
                model: None,
                additional_args: args(batch_id, Some(api_base)),
            },
            log.env,
        )
        .map(drop),
        CallEvent::Post { response } => {
            post_call(
                log.state,
                &PostCall {
                    original_response: response.clone(),
                    input,
                    additional_args: args(batch_id, None),
                },
                log.env,
            );
            Ok(())
        }
    }
}

fn args(batch_id: &str, api_base: Option<&str>) -> AdditionalArgs {
    let job_identifier = Json::Text(batch_id.to_owned());
    AdditionalArgs {
        api_base: api_base.map(str::to_owned),
        headers: None,
        complete_input_dict: Some(RawBody::Object(IndexMap::from([(
            "jobIdentifier".to_owned(),
            job_identifier,
        )]))),
        request_str: None,
        openai_sdk: None,
        passthrough: IndexMap::new(),
    }
}
