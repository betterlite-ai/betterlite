use std::error::Error;

use betterlite_logging::logging_pre_post::args::{AdditionalArgs, RawBody};
use betterlite_logging::logging_pre_post::pre_call::{PreCall, pre_call};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::RerankLogContext;
use super::error::RerankHttpError;
use crate::api::rerank_contracts::{Headers, Params};

// The pre-call logs the query, or an empty input when the caller sent none, and the body the transformer built
pub(super) fn log_pre_call<Failure>(
    api_base: &str,
    headers: &Headers,
    data: &Params,
    optional_params: &Params,
    log: &mut RerankLogContext<'_, '_>,
) -> Result<(), RerankHttpError<Failure>>
where
    Failure: Error + Send + Sync + 'static,
{
    let mut logged_headers = IndexMap::new();
    for (name, value) in headers {
        logged_headers.insert(name.clone(), Json::Text(value.clone()));
    }
    let mut logged_body = IndexMap::new();
    for (name, value) in data {
        logged_body.insert(name.clone(), value.clone());
    }
    let record = PreCall {
        input: optional_params
            .get("query")
            .cloned()
            .unwrap_or_else(|| Json::Text(String::new())),
        model: None,
        additional_args: AdditionalArgs {
            api_base: Some(api_base.to_owned()),
            headers: Some(logged_headers),
            complete_input_dict: Some(RawBody::Object(logged_body)),
            request_str: None,
            openai_sdk: None,
            passthrough: IndexMap::new(),
        },
    };
    pre_call(log.state, &record, log.env)?;
    Ok(())
}
