use betterlite_config::sections::guardrail_params::base::BaseLitellmParams;
use betterlite_config::sections::guardrail_params::content_filter::ContentFilterConfigModel;
use betterlite_config::sections::guardrail_params::decode::{
    decode_apply_request, decode_litellm_params, decode_patch_request,
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::OrHelperError as _;
use crate::helpers::{Helper, HelperResult};

pub(crate) struct Op;

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Function {
    BaseParams,
    LitellmParams,
    PatchRequest,
    ApplyRequest,
    ContentFilter,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    function: Function,
    body: Value,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let bytes = serde_json::to_vec(&input.body).or_value_error()?;
        let dump = match input.function {
            Function::BaseParams => serde_json::from_slice::<BaseLitellmParams>(&bytes)
                .ok()
                .and_then(|params| serde_json::to_value(params).ok()),
            Function::LitellmParams => decode_litellm_params(&bytes)
                .ok()
                .and_then(|params| serde_json::to_value(params).ok()),
            Function::PatchRequest => decode_patch_request(&bytes).ok().and_then(|request| {
                let sent = match request.litellm_params {
                    Some(params) => serde_json::to_value(params.sent_fields().ok()?).ok()?,
                    None => Value::Null,
                };
                Some(sent)
            }),
            Function::ApplyRequest => decode_apply_request(&bytes)
                .ok()
                .and_then(|request| serde_json::to_value(request).ok()),
            Function::ContentFilter => serde_json::from_slice::<ContentFilterConfigModel>(&bytes)
                .ok()
                .and_then(|model| serde_json::to_value(model).ok()),
        };
        Ok(dump.map_or_else(
            || json!({"accepted": false}),
            |accepted| json!({"accepted": true, "dump": accepted}),
        ))
    }
}
