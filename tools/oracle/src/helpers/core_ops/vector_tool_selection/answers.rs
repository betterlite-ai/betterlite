use betterlite_core::api::vector_tool_selection::error::VectorToolSelectionError;
use betterlite_core::api::vector_tool_selection::selection::{
    get_vector_store_by_name, get_vector_store_to_run, pop_recognised_tools,
    pop_stores_with_fallback,
};
use betterlite_openai::api::vector_store_records::records::vector::LiteLlmManagedVectorStore;
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::HelperResult;
use serde_json::{Value, json};
use tokio::runtime::{Builder, Runtime};

use super::HelperFailure;
use super::raised;
use super::scripted::{Input, Operation, ScriptedDatabase, decode_all, registry_of};
use super::values::{params_map, store_value, stores_value};

pub(super) fn drive(input: &Input) -> HelperResult {
    match input.operation {
        Operation::Consumed => consumed(input),
        Operation::Selection => selection(input),
        Operation::Lookup => lookup(input),
        Operation::Fallback => fallback(input),
    }
}

fn consumed(input: &Input) -> HelperResult {
    let registry = registry_of(decode_all(&input.stores)?);
    let mut tools = input.tools.clone().unwrap_or_default();
    let mut ids = input.ids.clone();
    let (params, raised_text) = match pop_recognised_tools(&registry, &mut tools, &mut ids) {
        Ok(records) => (params_map(&records), Value::Null),
        Err(error) => (json!({}), Value::String(error.to_string())),
    };
    Ok(json!({
        "params": params,
        "tools": tools,
        "ids": ids,
        "raised": raised_text,
    }))
}

fn selection(input: &Input) -> HelperResult {
    let mut registry = registry_of(decode_all(&input.stores)?);
    let mut params = input.non_default_params.clone();
    let mut tools = input.tools.clone();
    let runtime = runtime()?;
    let outcome = runtime.block_on(pop_stores_with_fallback(
        &mut registry,
        &mut params,
        tools.as_mut(),
        None::<&ScriptedDatabase>,
    ));
    let (stores, raised_text) = outcome_fields(outcome);
    Ok(json!({
        "stores": stores,
        "registry": stores_value(&registry.stores),
        "params": params,
        "tools": tools,
        "raised": raised_text,
    }))
}

fn lookup(input: &Input) -> HelperResult {
    let registry = registry_of(decode_all(&input.stores)?);
    let tools = input.tools.clone();
    let first = get_vector_store_to_run(&registry, &input.non_default_params, tools.as_deref())
        .or_value_error()?;
    let named = get_vector_store_by_name(&registry, &input.name);
    Ok(json!({
        "first": first.as_ref().map_or(Value::Null, store_value),
        "named": named.as_ref().map_or(Value::Null, store_value),
        "params": &input.non_default_params,
        "tools": tools,
        "registry": stores_value(&registry.stores),
    }))
}

fn fallback(input: &Input) -> HelperResult {
    let mut registry = registry_of(decode_all(&input.stores)?);
    let mut params = input.non_default_params.clone();
    let mut tools = input.tools.clone();
    let database = input
        .database
        .as_ref()
        .map(ScriptedDatabase::new)
        .transpose()?;
    let runtime = runtime()?;
    let outcome = runtime.block_on(pop_stores_with_fallback(
        &mut registry,
        &mut params,
        tools.as_mut(),
        database.as_ref(),
    ));
    let (stores, raised_text) = outcome_fields(outcome);
    let calls = database
        .as_ref()
        .map_or_else(Vec::new, ScriptedDatabase::calls);
    Ok(json!({
        "stores": stores,
        "registry": stores_value(&registry.stores),
        "params": params,
        "tools": tools,
        "database_calls": calls,
        "raised": raised_text,
    }))
}

fn outcome_fields(
    outcome: Result<Vec<LiteLlmManagedVectorStore>, VectorToolSelectionError>,
) -> (Value, Value) {
    match outcome {
        Ok(selected) => (stores_value(&selected), Value::Null),
        Err(error) => (json!([]), Value::String(error.to_string())),
    }
}

fn runtime() -> Result<Runtime, HelperFailure> {
    match Builder::new_current_thread().enable_all().build() {
        Ok(runtime) => Ok(runtime),
        Err(error) => Err(raised("RuntimeError", error.to_string())),
    }
}
