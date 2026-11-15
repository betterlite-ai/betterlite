use std::str::FromStr as _;

use betterlite_models::cost_usage::CostUsage;
use serde_json::{Map, Value};
use thiserror::Error;
use uuid::Uuid;

use crate::chat_choices::stream::{StreamError, chunk_parser};
use crate::chat_choices::wire::StreamReply;
use crate::xai_chat::response::WireUsage;
use crate::xai_usage::XaiUsage;
use crate::xai_usage::error::XaiUsageError;
use crate::xai_usage::reconcile::{
    fold_reasoning, normalize_totals, usage_restated_from_xai_ticks,
};

#[derive(Debug, Error)]
pub enum XaiStreamError {
    #[error(transparent)]
    Chunk(#[from] StreamError),
    #[error(transparent)]
    Usage(#[from] XaiUsageError),
}

// XAIChatCompletionStreamingHandler.chunk_parser: one empty choice for a usage-only chunk, then the parent parser
pub fn parse_chunk(
    chunk: &mut Value,
    identifier: fn() -> Uuid,
    clock: fn() -> i64,
) -> Result<StreamReply, XaiStreamError> {
    insert_empty_choice(chunk);
    reconcile_usage(chunk)?;
    let mut reply = chunk_parser(chunk, identifier, clock)?;
    restate_cost(&mut reply);
    Ok(reply)
}

fn insert_empty_choice(chunk: &mut Value) {
    let Value::Object(fields) = chunk else {
        return;
    };
    let empty = match fields.get("choices") {
        None => true,
        Some(Value::Array(items)) => items.is_empty(),
        Some(_) => false,
    };
    if !empty || !fields.contains_key("usage") {
        return;
    }
    let mut choice = Map::new();
    choice.insert(String::from("index"), Value::from(0_i64));
    choice.insert(String::from("delta"), Value::Object(Map::new()));
    choice.insert(String::from("finish_reason"), Value::Null);
    fields.insert(
        String::from("choices"),
        Value::Array(vec![Value::Object(choice)]),
    );
}

// the fold and total repair run on the raw usage block before the parent parser reads it
fn reconcile_usage(chunk: &mut Value) -> Result<(), XaiStreamError> {
    let Some(fields) = chunk
        .get("usage")
        .filter(|value| !value.is_null())
        .and_then(Value::as_object)
        .cloned()
    else {
        return Ok(());
    };
    let Ok(wire) = serde_json::from_value::<WireUsage>(Value::Object(fields)) else {
        tracing::debug!("the xAI stream usage did not have the declared shape; leaving it as sent");
        return Ok(());
    };
    let mut usage = wire.into_xai();
    fold_reasoning(&mut usage)?;
    normalize_totals(&mut usage)?;
    write_counts(chunk, &usage);
    Ok(())
}

fn write_counts(chunk: &mut Value, usage: &XaiUsage) {
    let Some(Value::Object(fields)) = chunk.get_mut("usage") else {
        return;
    };
    fields.insert(
        String::from("prompt_tokens"),
        Value::from(usage.tokens.prompt_tokens),
    );
    fields.insert(
        String::from("completion_tokens"),
        Value::from(usage.tokens.completion_tokens),
    );
    fields.insert(
        String::from("total_tokens"),
        Value::from(usage.tokens.total_tokens),
    );
}

fn restate_cost(reply: &mut StreamReply) {
    let Some(Value::Object(fields)) = reply.usage.as_mut() else {
        return;
    };
    let carrier = XaiUsage {
        tokens: CostUsage::default(),
        cost: None,
        cost_in_usd_ticks: fields.get("cost_in_usd_ticks").cloned(),
        server_side_tool_usage_details: None,
    };
    let Some(cost) = usage_restated_from_xai_ticks(&carrier).and_then(|restated| restated.cost)
    else {
        return;
    };
    let Ok(number) = serde_json::Number::from_str(&cost.normalize().to_string()) else {
        return;
    };
    fields.insert(String::from("cost"), Value::Number(number));
}
