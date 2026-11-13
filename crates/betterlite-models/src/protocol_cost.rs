pub mod error;
mod money;

use indexmap::IndexMap;
use rust_decimal::Decimal;

use crate::decisions::Json;
use error::ProtocolCostError;
use money::{Strings, money};

mod tests;

// What the A2A calculator can read from the usage record. A dictionary has no token attributes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum A2aUsage {
    Absent,
    Mapping,
    Tokens {
        prompt_tokens: i64,
        completion_tokens: i64,
    },
}

// Json::Null stands for a missing or null value, which is what Python's `is not None` checks skip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct A2aCall {
    pub response_cost: Json,
    pub cost_per_query: Json,
    pub input_cost_per_token: Json,
    pub output_cost_per_token: Json,
    pub usage: A2aUsage,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpCall {
    pub response_cost: Json,
    pub tool_name: String,
    pub default_cost_per_query: Json,
    pub tool_name_to_cost_per_query: IndexMap<String, Json>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProtocolCall {
    A2a(A2aCall),
    Mcp(McpCall),
}

pub fn protocol_charge(call: &ProtocolCall) -> Result<Decimal, ProtocolCostError> {
    match call {
        ProtocolCall::A2a(agent) => agent_charge(agent),
        ProtocolCall::Mcp(tool) => tool_charge(tool),
    }
}

fn agent_charge(call: &A2aCall) -> Result<Decimal, ProtocolCostError> {
    if let Some(cost) = money("response_cost", &call.response_cost, Strings::Float)? {
        return Ok(cost);
    }
    if let Some(cost) = money("cost_per_query", &call.cost_per_query, Strings::Float)? {
        return Ok(cost);
    }
    if call.input_cost_per_token == Json::Null && call.output_cost_per_token == Json::Null {
        return Ok(Decimal::ZERO);
    }
    let (prompt_tokens, completion_tokens) = match call.usage {
        A2aUsage::Absent => return Ok(Decimal::ZERO),
        A2aUsage::Mapping => (0, 0),
        A2aUsage::Tokens {
            prompt_tokens,
            completion_tokens,
        } => (prompt_tokens, completion_tokens),
    };
    let input_rate = money(
        "input_cost_per_token",
        &call.input_cost_per_token,
        Strings::Float,
    )?
    .unwrap_or(Decimal::ZERO);
    let output_rate = money(
        "output_cost_per_token",
        &call.output_cost_per_token,
        Strings::Float,
    )?
    .unwrap_or(Decimal::ZERO);
    Decimal::from(prompt_tokens)
        .checked_mul(input_rate)
        .zip(Decimal::from(completion_tokens).checked_mul(output_rate))
        .and_then(|(input_cost, output_cost)| input_cost.checked_add(output_cost))
        .ok_or(ProtocolCostError::Overflow)
}

fn tool_charge(call: &McpCall) -> Result<Decimal, ProtocolCostError> {
    if let Some(cost) = money("response_cost", &call.response_cost, Strings::Reject)? {
        return Ok(cost);
    }
    // A listed tool decides the price even when the listed value is null.
    if let Some(listed) = call.tool_name_to_cost_per_query.get(&call.tool_name) {
        return money("tool_name_to_cost_per_query", listed, Strings::Reject)?.ok_or(
            ProtocolCostError::InvalidMoney {
                field: "tool_name_to_cost_per_query",
            },
        );
    }
    Ok(money(
        "default_cost_per_query",
        &call.default_cost_per_query,
        Strings::Reject,
    )?
    .unwrap_or(Decimal::ZERO))
}
