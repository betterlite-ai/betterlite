use betterlite_core::api::vector_store_registry::store::VectorStoreRegistry;
use betterlite_core::api::vector_tool_selection::error::VectorToolSelectionError;
use betterlite_core::api::vector_tool_selection::selection::{
    get_vector_store_by_name, get_vector_store_to_run, pop_recognised_tools,
    pop_stores_with_fallback,
};
use betterlite_models::model_registration::JsonObject;
use betterlite_openai::api::vector_store_records::records::vector::LiteLlmManagedVectorStore;
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::HelperResult;
use serde::Serialize;
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
    let registry = stored_registry(input)?;
    consumed_answer(&registry, input)
}

fn consumed_answer(registry: &VectorStoreRegistry, input: &Input) -> HelperResult {
    let mut tools = input.tools.clone().unwrap_or_default();
    let mut ids = input.ids.clone();
    let (params, raised_text) = match pop_recognised_tools(registry, &mut tools, &mut ids) {
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

fn lookup(input: &Input) -> HelperResult {
    let registry = stored_registry(input)?;
    lookup_answer(&registry, input, input.tools.clone())
}

fn lookup_answer(
    registry: &VectorStoreRegistry,
    input: &Input,
    tools: Option<Vec<JsonObject>>,
) -> HelperResult {
    let first = get_vector_store_to_run(registry, &input.non_default_params, tools.as_deref())
        .or_value_error()?;
    let named = get_vector_store_by_name(registry, &input.name);
    let fields = LookupAnswer {
        first: first.as_ref().map_or(Value::Null, store_value),
        named: named.as_ref().map_or(Value::Null, store_value),
        params: &input.non_default_params,
        tools: &tools,
        registry: stores_value(&registry.stores),
    };
    Ok(json!(fields))
}

// the two lookups answer as a record, so the shape no longer reads like the selection answer
#[derive(Serialize)]
struct LookupAnswer<'answer> {
    first: Value,
    named: Value,
    params: &'answer JsonObject,
    tools: &'answer Option<Vec<JsonObject>>,
    registry: Value,
}

fn selection(input: &Input) -> HelperResult {
    let selected = select_stores(input, None)?;
    Ok(answer(&selected, None))
}

fn fallback(input: &Input) -> HelperResult {
    let database = input
        .database
        .as_ref()
        .map(ScriptedDatabase::new)
        .transpose()?;
    let selected = select_stores(input, database.as_ref())?;
    let calls = database
        .as_ref()
        .map_or_else(Vec::new, ScriptedDatabase::calls);
    Ok(answer(&selected, Some(calls)))
}

fn stored_registry(input: &Input) -> Result<VectorStoreRegistry, HelperFailure> {
    let stores = decode_all(&input.stores)?;
    Ok(registry_of(stores))
}

struct Selected {
    registry: VectorStoreRegistry,
    params: JsonObject,
    tools: Option<Vec<JsonObject>>,
    stores: Value,
    raised: Value,
}

fn select_stores(
    input: &Input,
    database: Option<&ScriptedDatabase>,
) -> Result<Selected, HelperFailure> {
    let mut registry = stored_registry(input)?;
    let mut params = input.non_default_params.clone();
    let mut tools = input.tools.clone();
    let runtime = runtime()?;
    let outcome = runtime.block_on(pop_stores_with_fallback(
        &mut registry,
        &mut params,
        tools.as_mut(),
        database,
    ));
    let (stores, raised_text) = outcome_fields(outcome);
    Ok(Selected {
        registry,
        params,
        tools,
        stores,
        raised: raised_text,
    })
}

// the selection answer omits database_calls and the fallback answer carries it, as the two LiteLLM calls return different shapes
#[derive(Serialize)]
struct Answer<'answer> {
    stores: &'answer Value,
    registry: Value,
    params: &'answer JsonObject,
    tools: &'answer Option<Vec<JsonObject>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    database_calls: Option<Vec<Value>>,
    raised: &'answer Value,
}

fn answer(selected: &Selected, database_calls: Option<Vec<Value>>) -> Value {
    let fields = Answer {
        stores: &selected.stores,
        registry: stores_value(&selected.registry.stores),
        params: &selected.params,
        tools: &selected.tools,
        database_calls,
        raised: &selected.raised,
    };
    json!(fields)
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
