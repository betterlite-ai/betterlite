use betterlite_logging::logging_pre_post::args::{AdditionalArgs, RawBody};
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_logging::logging_pre_post::masking::masked_headers;
use betterlite_logging::logging_pre_post::post_call::{PostCall, post_call};
use betterlite_logging::logging_pre_post::pre_call::{PreCall, pre_call};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::image_call::plan::LogContext;

// LiteLLM passes the invocation ARN into both callbacks under complete_input_dict
fn complete_input_dict(invocation_arn: &str) -> RawBody {
    RawBody::Object(IndexMap::from([(
        "invocation_arn".to_owned(),
        Json::Text(invocation_arn.to_owned()),
    )]))
}

fn live_headers(headers: &[(String, String)]) -> IndexMap<String, Json> {
    headers
        .iter()
        .map(|(name, value)| (name.clone(), Json::Text(value.clone())))
        .collect()
}

// the curl view masks the signed headers but keeps the prepared order
fn curl_request(url: &str, headers: &[(String, String)]) -> String {
    let masked = masked_headers(&live_headers(headers), false);
    let formatted: Vec<String> = masked
        .iter()
        .map(|(name, value)| format!("-H '{name}: {}'", value.as_str().unwrap_or_default()))
        .collect();
    format!(
        "\n\nGET Request Sent from LiteLLM:\ncurl -X GET \\\n{url} \\\n{}\n",
        formatted.join(" ")
    )
}

pub(crate) fn log_pre_call(
    invocation_arn: &str,
    url: &str,
    headers: &[(String, String)],
    log: &mut LogContext<'_, '_>,
) -> Result<(), PrePostError> {
    pre_call(
        log.state,
        &PreCall {
            input: Json::Text(invocation_arn.to_owned()),
            model: None,
            additional_args: AdditionalArgs {
                api_base: Some(url.to_owned()),
                headers: Some(live_headers(headers)),
                complete_input_dict: Some(complete_input_dict(invocation_arn)),
                request_str: Some(curl_request(url, headers)),
                openai_sdk: None,
                passthrough: IndexMap::new(),
            },
        },
        log.env,
    )?;
    Ok(())
}

pub(crate) fn log_post_call(invocation_arn: &str, text: &str, log: &mut LogContext<'_, '_>) {
    let mut additional = AdditionalArgs::default();
    additional.complete_input_dict = Some(complete_input_dict(invocation_arn));
    post_call(
        log.state,
        &PostCall {
            original_response: Json::Text(text.to_owned()),
            input: Json::Text(invocation_arn.to_owned()),
            additional_args: additional,
        },
        log.env,
    );
}
