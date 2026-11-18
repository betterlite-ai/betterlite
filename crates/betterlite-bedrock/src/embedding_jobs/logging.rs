use betterlite_logging::clickhouse::clickhouse_logging::convert::coerced_text;
use betterlite_logging::logging_pre_post::args::{AdditionalArgs, RawBody};
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_logging::logging_pre_post::masking::masked_headers;
use betterlite_logging::logging_pre_post::post_call::{PostCall, post_call};
use betterlite_logging::logging_pre_post::pre_call::{PreCall, pre_call};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::image_call::plan::LogContext;

// the curl view masks the signed headers but keeps the prepared order
fn curl_request(url: &str, live: &IndexMap<String, Json>) -> String {
    let masked = masked_headers(live, false);
    let formatted: Vec<String> = masked
        .iter()
        .map(|(name, value)| format!("-H '{name}: {}'", coerced_text(value)))
        .collect();
    format!(
        "\n\nGET Request Sent from LiteLLM:\ncurl -X GET \\\n{url} \\\n{}\n",
        formatted.join(" ")
    )
}

pub(crate) enum StatusEvent<'event> {
    Request {
        url: &'event str,
        headers: &'event [(String, String)],
    },
    Reply(&'event str),
}

pub(crate) fn log_call(
    invocation_arn: &str,
    event: StatusEvent<'_>,
    log: &mut LogContext<'_, '_>,
) -> Result<(), PrePostError> {
    let request = match &event {
        StatusEvent::Request { url, headers } => Some((*url, *headers)),
        StatusEvent::Reply(_) => None,
    };
    let additional_args = call_args(invocation_arn, request);
    let input = Json::Text(invocation_arn.to_owned());
    match event {
        StatusEvent::Request { .. } => {
            pre_call(
                log.state,
                &PreCall {
                    input,
                    model: None,
                    additional_args,
                },
                log.env,
            )?;
        }
        StatusEvent::Reply(text) => post_call(
            log.state,
            &PostCall {
                original_response: Json::Text(text.to_owned()),
                input,
                additional_args,
            },
            log.env,
        ),
    }
    Ok(())
}

fn call_args(invocation_arn: &str, request: Option<(&str, &[(String, String)])>) -> AdditionalArgs {
    let mut args = AdditionalArgs {
        api_base: None,
        headers: None,
        complete_input_dict: Some(RawBody::Object(IndexMap::from([(
            "invocation_arn".to_owned(),
            Json::Text(invocation_arn.to_owned()),
        )]))),
        request_str: None,
        openai_sdk: None,
        passthrough: IndexMap::new(),
    };
    if let Some((url, headers)) = request {
        let live: IndexMap<String, Json> = headers
            .iter()
            .map(|(name, value)| (name.clone(), Json::Text(value.clone())))
            .collect();
        args.request_str = Some(curl_request(url, &live));
        args.api_base = Some(url.to_owned());
        args.headers = Some(live);
    }
    args
}
