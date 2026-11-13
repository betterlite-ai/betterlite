use std::collections::BTreeMap;

use betterlite_config::replicate_price::load_replicate_gpu_price;
use betterlite_core::cost::special::error::ReplicateError;
use betterlite_core::cost::special::replicate::replicate_charge;
use betterlite_models::decisions::Json;
use betterlite_models::protocol_cost::error::ProtocolCostError;
use betterlite_models::protocol_cost::{A2aCall, A2aUsage, McpCall, ProtocolCall, protocol_charge};
use indexmap::IndexMap;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use serde::Deserialize;
use serde_json::{Number, json};
use thiserror::Error;

pub(crate) struct Op;

// The tag names the call and the field next to it carries its record; a null record is a missing logging object.
#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    A2a { a2a: Option<A2aInput> },
    Mcp { mcp: Option<McpInput> },
    Replicate { replicate_charge: ReplicateInput },
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum UsageKind {
    Absent,
    Mapping,
    Tokens,
}

#[derive(Debug, Deserialize)]
pub(crate) struct A2aInput {
    response_cost: Json,
    cost_per_query: Json,
    input_cost_per_token: Json,
    output_cost_per_token: Json,
    usage: UsageKind,
    prompt_tokens: i64,
    completion_tokens: i64,
}

#[derive(Debug, Deserialize)]
pub(crate) struct McpInput {
    response_cost: Json,
    tool_name: String,
    default_cost_per_query: Json,
    tool_name_to_cost_per_query: IndexMap<String, Json>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ReplicateInput {
    total_time: Number,
}

#[derive(Debug, Error)]
enum Failure {
    #[error(transparent)]
    Protocol(#[from] ProtocolCostError),
    #[error(transparent)]
    Replicate(#[from] ReplicateError),
    #[error("invalid total_time: {0}")]
    TotalTime(String),
    #[error(transparent)]
    Price(#[from] betterlite_config::loader::ConfigError),
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let cost = match input {
            Input::A2a { a2a: None } | Input::Mcp { mcp: None } => Ok(Decimal::ZERO),
            Input::A2a { a2a: Some(agent) } => {
                protocol_charge(&ProtocolCall::from(agent)).map_err(Failure::from)
            }
            Input::Mcp { mcp: Some(tool) } => protocol_charge(&ProtocolCall::Mcp(McpCall {
                response_cost: tool.response_cost.clone(),
                tool_name: tool.tool_name.clone(),
                default_cost_per_query: tool.default_cost_per_query.clone(),
                tool_name_to_cost_per_query: tool.tool_name_to_cost_per_query.clone(),
            }))
            .map_err(Failure::from),
            Input::Replicate { replicate_charge } => replicate_cost(replicate_charge),
        }
        .map_err(Box::<crate::helpers::HelperError>::from)?;
        let mut output = json!({"cost": cost});
        canonical_numbers(&mut output);
        Ok(output)
    }
}

impl From<&A2aInput> for ProtocolCall {
    fn from(agent: &A2aInput) -> Self {
        let usage = match agent.usage {
            UsageKind::Absent => A2aUsage::Absent,
            UsageKind::Mapping => A2aUsage::Mapping,
            UsageKind::Tokens => A2aUsage::Tokens {
                prompt_tokens: agent.prompt_tokens,
                completion_tokens: agent.completion_tokens,
            },
        };
        Self::A2a(A2aCall {
            response_cost: agent.response_cost.clone(),
            cost_per_query: agent.cost_per_query.clone(),
            input_cost_per_token: agent.input_cost_per_token.clone(),
            output_cost_per_token: agent.output_cost_per_token.clone(),
            usage,
        })
    }
}

fn replicate_cost(replicate: &ReplicateInput) -> Result<Decimal, Failure> {
    let price = load_replicate_gpu_price(&BTreeMap::<String, String>::new())?;
    let total_time = Decimal::from_str_exact(&replicate.total_time.to_string())
        .map_err(|error| Failure::TotalTime(error.to_string()))?;
    // LiteLLM reads the clock for a zero total_time, so the cases only send a nonzero one.
    Ok(replicate_charge(
        price,
        total_time,
        Decimal::ZERO,
        Decimal::ZERO,
    )?)
}

fn canonical_numbers(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(fields) => {
            for nested in fields.values_mut() {
                canonical_numbers(nested);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                canonical_numbers(item);
            }
        }
        serde_json::Value::Number(number) => {
            if let Some(float) = number.as_f64()
                && let Some(canonical) = Number::from_f64(float)
            {
                *number = canonical;
            }
        }
        serde_json::Value::String(text) => {
            if let Ok(decimal) = Decimal::from_str_exact(text)
                && let Some(float) = decimal.to_f64()
                && let Some(canonical) = Number::from_f64(float)
            {
                *value = serde_json::Value::Number(canonical);
            }
        }
        serde_json::Value::Null | serde_json::Value::Bool(_) => {}
    }
}

impl From<Failure> for Box<crate::helpers::HelperError> {
    fn from(failure: Failure) -> Self {
        Self::new(crate::helpers::HelperError {
            class: "ValueError".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: failure.to_string(),
        })
    }
}
