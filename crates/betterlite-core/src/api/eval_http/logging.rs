use betterlite_logging::logging_pre_post::args::{AdditionalArgs, RawBody};
use betterlite_logging::logging_pre_post::pre_call::{PreCall, pre_call};
use betterlite_models::decisions::Json;
use betterlite_openai::openai_base_config::{Headers, Object};
use indexmap::IndexMap;

use super::error::EvalHttpError;
use super::exchange::{EvalConnection, EvalExchange, EvalLogging};

fn logged_headers(headers: &Headers) -> IndexMap<String, Json> {
    headers
        .iter()
        .map(|(name, value)| (name.clone(), Json::Text(value.clone())))
        .collect()
}

// The pre-call keeps the body or query as JSON objects, the shape LiteLLM's dict has when it reaches a callback
fn complete_input_dict(fields: &Object) -> Result<RawBody, EvalHttpError> {
    let converted = serde_json::to_value(fields)
        .and_then(serde_json::from_value::<IndexMap<String, Json>>)
        .map_err(|source| EvalHttpError::Record { source })?;
    Ok(RawBody::Object(converted))
}

// LiteLLM's pre_call with api_key "": its input, the URL, the validated headers and the body or query it logs
pub(super) fn record_pre_call(
    connection: &EvalConnection<'_>,
    exchange: &EvalExchange<'_>,
    log: &mut EvalLogging<'_, '_>,
) -> Result<(), EvalHttpError> {
    // list_runs logs its query under `params`, beside the fields every eval pre-call records
    let passthrough = match exchange.params {
        Some(query) => {
            let params = complete_input_dict(query)?;
            IndexMap::from([("params".to_owned(), Json::from(&params))])
        }
        None => IndexMap::new(),
    };
    let record = PreCall {
        input: Json::Text(exchange.input.to_owned()),
        model: None,
        additional_args: AdditionalArgs {
            api_base: Some(connection.url.to_owned()),
            headers: Some(logged_headers(connection.headers)),
            complete_input_dict: exchange
                .complete_input_dict
                .map(complete_input_dict)
                .transpose()?,
            request_str: None,
            openai_sdk: None,
            passthrough,
        },
    };
    pre_call(log.state, &record, log.env)?;
    Ok(())
}
