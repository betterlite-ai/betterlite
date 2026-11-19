use std::sync::Arc;

use betterlite_auth::auth_model_lists::credential::CredentialSnapshot;
use betterlite_config::api::vector_store_registry::RegistryEntry;
use betterlite_core::api::vector_store_registry::index::VectorStoreIndexRegistry;
use betterlite_core::api::vector_store_registry::repository::{
    indexes_from_database, stores_from_database,
};
use betterlite_core::api::vector_store_registry::store::VectorStoreRegistry;
use betterlite_core::api::vector_store_registry::tools::{
    VectorStoreToolParams, extract_tool_params, ids_to_run,
};
use betterlite_models::model_registration::JsonObject;
use betterlite_openai::api::vector_store_records::records::vector::LiteLlmManagedVectorStore;
use betterlite_oracle_helper_api::helper::HelperResult;
use chrono::Utc;
use serde_json::{Value, json};
use tokio::runtime::{Builder, Runtime};

use super::HelperFailure;
use super::raised;
use super::scripted::{
    Input, Operation, ScriptedCredentials, ScriptedDatabase, Step, decode, decode_all,
};
use super::values::{index_value, indexes_value, object_value, store_value, stores_value};

pub(super) fn drive(input: &Input) -> HelperResult {
    match input.operation {
        Operation::ConfigRecords => config_records(input),
        Operation::Mutations => mutations(input),
        Operation::DatabaseLookup => database_lookup(input),
        Operation::DatabaseRows => database_rows(input),
        Operation::IdsAndToolParams => ids_and_tool_params(input),
        Operation::NamedCredentials => named_credentials(input),
    }
}

fn config_records(input: &Input) -> HelperResult {
    let entries = decode_all::<RegistryEntry>(&input.entries)?;
    let mut registry = VectorStoreRegistry::default();
    let raised_text = match registry.load_vector_stores_from_config(&entries) {
        Ok(()) => Value::Null,
        Err(error) => Value::String(error.to_string()),
    };
    Ok(json!({
        "stores": stores_value(&registry.stores),
        "raised": raised_text,
    }))
}

fn mutations(input: &Input) -> HelperResult {
    let mut registry = registry_with(decode_all(&input.stores)?);
    let mut index_registry = VectorStoreIndexRegistry::new(decode_all(&input.indexes)?);
    let mut results = Vec::with_capacity(input.steps.len());
    for step in &input.steps {
        results.push(step_value(step, &mut registry, &mut index_registry)?);
    }
    Ok(json!({"results": results}))
}

fn step_value(
    step: &Step,
    registry: &mut VectorStoreRegistry,
    index_registry: &mut VectorStoreIndexRegistry,
) -> HelperResult {
    match step {
        Step::Add { store } => {
            registry.add_store(decode(store)?);
            Ok(json!({"stores": stores_value(&registry.stores)}))
        }
        Step::Update { id, store } => {
            registry.update_store(id, decode(store)?);
            Ok(json!({"stores": stores_value(&registry.stores)}))
        }
        Step::Delete { id } => {
            registry.remove_store(id);
            Ok(json!({"stores": stores_value(&registry.stores)}))
        }
        Step::IsConfig { id } => Ok(json!({"is_config": registry.is_config(id)})),
        Step::UpsertIndex { index } => {
            index_registry.upsert(decode(index)?);
            Ok(json!({"indexes": indexes_value(&index_registry.indexes)}))
        }
        Step::LookupIndex { name } => {
            let found = index_registry
                .find_by_name(name)
                .map_or(Value::Null, index_value);
            Ok(json!({"index": found}))
        }
        Step::IsIndex { name } => Ok(json!({"is_index": index_registry.contains_name(name)})),
        Step::Defaults => {
            let stores = VectorStoreRegistry::default();
            let indexes = VectorStoreIndexRegistry::default();
            Ok(json!({
                "stores": stores_value(&stores.stores),
                "indexes": indexes_value(&indexes.indexes),
            }))
        }
    }
}

fn database_lookup(input: &Input) -> HelperResult {
    let mut registry = registry_with(decode_all(&input.stores)?);
    let database = input
        .database
        .as_ref()
        .map(ScriptedDatabase::new)
        .transpose()?;
    let runtime = runtime()?;
    let mut results = Vec::with_capacity(input.lookups.len());
    for lookup in &input.lookups {
        let found = runtime.block_on(registry.lookup_or_database(lookup, database.as_ref()));
        results.push(found.as_ref().map_or(Value::Null, store_value));
    }
    let calls = database
        .as_ref()
        .map_or_else(Vec::new, ScriptedDatabase::calls);
    Ok(json!({"results": results, "database_calls": calls}))
}

fn database_rows(input: &Input) -> HelperResult {
    let database = input
        .database
        .as_ref()
        .map(ScriptedDatabase::new)
        .transpose()?;
    let runtime = runtime()?;
    let stores = runtime.block_on(stores_from_database(database.as_ref()));
    let indexes = runtime.block_on(indexes_from_database(database.as_ref()));
    let stores_failed = stores.is_err();
    let indexes_failed = indexes.is_err();
    let store_rows = match stores {
        Ok(rows) => stores_value(&rows),
        Err(_) => Value::Array(Vec::new()),
    };
    let index_rows = match indexes {
        Ok(rows) => indexes_value(&rows),
        Err(_) => Value::Array(Vec::new()),
    };
    let calls = database
        .as_ref()
        .map_or_else(Vec::new, ScriptedDatabase::calls);
    Ok(json!({
        "stores": store_rows,
        "stores_failed": stores_failed,
        "indexes": index_rows,
        "indexes_failed": indexes_failed,
        "database_calls": calls,
    }))
}

fn ids_and_tool_params(input: &Input) -> HelperResult {
    let ids = ids_to_run(&input.non_default_params, input.tools.as_deref())?;
    let (params, raised_text) = tool_params_fields(input.tool.as_ref());
    Ok(json!({
        "ids": ids,
        "params": &input.non_default_params,
        "tools": &input.tools,
        "tool_params": params,
        "raised": raised_text,
    }))
}

fn tool_params_fields(tool: Option<&JsonObject>) -> (Value, Value) {
    let empty = JsonObject::new();
    let tool = tool.unwrap_or(&empty);
    match extract_tool_params(tool) {
        Ok(params) => (tool_params_record(&params), Value::Null),
        Err(error) => (Value::Null, Value::String(error.to_string())),
    }
}

fn tool_params_record(params: &VectorStoreToolParams) -> Value {
    let results = params
        .max_num_results
        .map_or(Value::Null, |results| json!(results));
    json!({
        "filters": object_value(params.filters.as_ref()),
        "max_num_results": results,
        "ranking_options": object_value(params.ranking_options.as_ref()),
    })
}

fn named_credentials(input: &Input) -> HelperResult {
    let stores = decode_all::<LiteLlmManagedVectorStore>(&input.stores)?;
    let credentials = Arc::new(ScriptedCredentials {
        values: input.credentials.clone(),
    });
    let registry = VectorStoreRegistry::new(stores, Arc::new(Utc::now), credentials);
    let mut results = Vec::with_capacity(input.ids.len());
    for id in &input.ids {
        results.push(json!(registry.credentials_for_vector_store(id)));
    }
    Ok(json!({"credentials": results}))
}

fn registry_with(stores: Vec<LiteLlmManagedVectorStore>) -> VectorStoreRegistry {
    VectorStoreRegistry::new(
        stores,
        Arc::new(Utc::now),
        Arc::new(CredentialSnapshot::default()),
    )
}

fn runtime() -> Result<Runtime, HelperFailure> {
    match Builder::new_current_thread().enable_all().build() {
        Ok(runtime) => Ok(runtime),
        Err(error) => Err(raised("RuntimeError", error.to_string())),
    }
}
