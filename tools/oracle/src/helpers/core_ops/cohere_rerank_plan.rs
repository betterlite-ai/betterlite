use std::collections::BTreeMap;

use betterlite_config::sections::cohere_rerank_settings::Settings;
use betterlite_openai::api::responses_shapes::wire::WireDecodeLimits;
use betterlite_openai::cohere_rerank::{
    CohereRerankError, RerankParams, RerankVersion, complete_url, map_rerank_params,
    request_headers, transform_rerank_request, transform_rerank_response,
};
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::json;

use crate::helpers::{Helper, HelperError, HelperResult, OrHelperError as _};

mod tests;

const LIMITS: WireDecodeLimits = WireDecodeLimits {
    max_json_bytes: 0x0010_0000,
    max_event_bytes: 0x0004_0000,
    max_items: 10_000,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Call {
    version: RerankVersion,
    model: String,
    api_base: Option<String>,
    api_key: Option<String>,
    headers: BTreeMap<String, String>,
    settings: Settings,
    options: RerankParams,
    non_default: RerankParams,
    responses: Vec<Reply>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Reply {
    status: u16,
    body: String,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let mut answers = Vec::with_capacity(input.calls.len());
        for call in &input.calls {
            let key = call.api_key.as_deref().map(SecretString::from);
            let mapped = map_rerank_params(call.version, &call.options, &call.non_default);
            let headers =
                request_headers(&call.headers, key.as_ref(), &call.settings).or_value_error()?;
            let body =
                transform_rerank_request(call.version, &call.model, &mapped).or_value_error()?;
            for reply in &call.responses {
                match transform_rerank_response(reply.status, reply.body.as_bytes(), LIMITS) {
                    Err(CohereRerankError::ResponseJson { status, message }) => {
                        return Err(Box::new(HelperError {
                            class: "CohereError".to_owned(),
                            status: Some(status),
                            error_type: Some(None),
                            param: Some(None),
                            code: Some(None),
                            message,
                        }));
                    }
                    other => other.map(drop).or_value_error()?,
                }
            }
            answers.push(json!({
                "mapped": mapped,
                "url": complete_url(call.version, call.api_base.as_deref()),
                "headers": headers,
                "body": body,
            }));
        }
        Ok(json!(answers))
    }
}
