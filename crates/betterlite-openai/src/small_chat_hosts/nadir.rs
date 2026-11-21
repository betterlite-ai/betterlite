use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::HiddenParams;
use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive as _;
use serde_json::Value;

use crate::chat_choices::reply::{
    ResponseReply, TransformResponseError, transform_response as parent_transform_response,
};
use crate::compatible_wire::{LogInput, ReplyOptions};
use crate::edenai_common::set_response_cost;
use crate::openai_base_config::Headers;
use crate::openai_base_config::forwarding::ForwardingFailure;

const SUPPORTED_PARAMS: [&str; 9] = [
    "extra_headers",
    "frequency_penalty",
    "max_retries",
    "max_tokens",
    "presence_penalty",
    "response_format",
    "stream",
    "temperature",
    "top_p",
];

const METADATA: &str = "nadir_metadata";
const COST: &str = "cost";
const TOTAL_COST_USD: &str = "total_cost_usd";

// NadirConfig redefines its supported list and its reply transform
#[derive(Debug, Clone, Copy, Default)]
pub struct NadirConfig;

pub struct ReplyRequest<'request, 'call, 'env> {
    pub reply: ResponseReply<'request>,
    pub state: &'request mut LoggingState,
    pub env: &'request PrePostEnv<'env>,
    pub input: LogInput,
    pub options: ReplyOptions<'call>,
}

pub struct NadirReply {
    pub response: Value,
    pub hidden_params: HiddenParams,
}

impl NadirConfig {
    #[must_use]
    pub fn supported_params() -> Vec<&'static str> {
        SUPPORTED_PARAMS.to_vec()
    }
}

// LiteLLM's _reported_cost_usd: the cost at nadir_metadata.cost.total_cost_usd, and every
// other shape is no cost. A boolean is an int in Python, so it is rejected before the number
// check, and an integer counts and becomes a float.
#[must_use]
pub fn nadir_reported_cost(bytes: &[u8]) -> Option<f64> {
    let payload: Value = serde_json::from_slice(bytes).ok()?;
    let cost = payload.get(METADATA)?.get(COST)?.get(TOTAL_COST_USD)?;
    if cost.is_boolean() {
        return None;
    }
    let number = cost.as_f64()?;
    if !number.is_finite() || number < 0.0_f64 {
        return None;
    }
    Some(number)
}

// the parent's reply transform runs first; a failure returns there and the bytes are never read
pub fn transform_response<Projection>(
    request: ReplyRequest<'_, '_, '_>,
    projection: Projection,
) -> Result<NadirReply, TransformResponseError>
where
    Projection: FnOnce(Value, &Headers, &Headers) -> Result<Value, ForwardingFailure>,
{
    let ReplyRequest {
        reply,
        state,
        env,
        input,
        options,
    } = request;
    let bytes = reply.body;
    let response = parent_transform_response(&reply, state, env, input, projection)?;
    let mut hidden_params = HiddenParams::new();
    set_response_cost(
        &mut hidden_params,
        nadir_reported_cost(bytes).and_then(Decimal::from_f64),
    );
    Ok(NadirReply {
        response,
        hidden_params,
    })
}
