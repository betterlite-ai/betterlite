#![cfg(test)]

mod postgres;
mod support;

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_openai::api::vector_store_records::records::vector::LiteLlmManagedVectorStore;

use crate::api::vector_store_registry::error::VectorStoreRegistryError;
use crate::api::vector_store_registry::store::VectorStoreRegistry;

use super::error::VectorToolSelectionError;
use super::selection::{
    get_vector_store_by_name, get_vector_store_to_run, pop_recognised_tools,
    pop_stores_with_fallback,
};
use support::{
    CapturedLogs, ScriptedDatabase, copied, list, object, registry, store, text, with_params,
};

fn number(value: i64) -> Json {
    Json::Number(serde_json::Number::from(value))
}

fn params_json(fields: JsonObject) -> Json {
    Json::Object(fields)
}

fn tool(id: &str, extra: Vec<(&str, Json)>) -> JsonObject {
    let mut fields = object(vec![
        ("type", text("file_search")),
        ("vector_store_ids", list(&[id])),
    ]);
    for (name, value) in extra {
        fields.insert(name.to_owned(), value);
    }
    fields
}

#[tokio::test]
async fn recognized_tools_are_consumed() {
    recognized_tools_leave_unmatched_tools_whole();
    a_later_tool_replaces_the_record();
    a_tool_without_ids_stays();
    missing_and_empty_tools_change_nothing().await;
    wrong_id_types_name_their_field().await;
}

fn a_tool_without_ids_stays() {
    let registry = registry(vec![store("vs_a")]);
    let mut tools = vec![object(vec![
        ("type", text("function")),
        ("name", text("weather")),
    ])];
    let mut ids = Vec::new();
    let params = pop_recognised_tools(&registry, &mut tools, &mut ids).unwrap();
    assert!(params.is_empty());
    assert_eq!(tools.len(), 1);
    assert!(ids.is_empty());
}

fn recognized_tools_leave_unmatched_tools_whole() {
    let registry = registry(vec![store("vs_a"), store("vs_b")]);
    let mut tools = vec![
        object(vec![
            ("type", text("custom")),
            ("vector_store_ids", list(&["vs_a"])),
            ("max_num_results", number(2)),
        ]),
        object(vec![
            ("type", text("file_search")),
            ("vector_store_ids", list(&["vs_a", "unknown"])),
        ]),
        object(vec![
            ("type", text("file_search")),
            ("vector_store_ids", list(&[])),
        ]),
    ];
    let mut ids = vec!["vs_request".to_owned()];
    let params = pop_recognised_tools(&registry, &mut tools, &mut ids).unwrap();
    assert_eq!(tools.len(), 2);
    assert_eq!(
        tools[0].get("vector_store_ids"),
        Some(&list(&["vs_a", "unknown"]))
    );
    assert_eq!(ids, vec!["vs_request", "vs_a"]);
    assert_eq!(params.len(), 1);
    assert_eq!(params.get("vs_a").unwrap().max_num_results, Some(2));
}

fn a_later_tool_replaces_the_record() {
    let registry = registry(vec![store("vs_a")]);
    let mut tools = vec![
        tool("vs_a", vec![("max_num_results", number(2))]),
        tool(
            "vs_a",
            vec![("filters", params_json(object(vec![("tool", text("x"))])))],
        ),
    ];
    let mut ids = vec!["vs_b".to_owned()];
    let params = pop_recognised_tools(&registry, &mut tools, &mut ids).unwrap();
    let record = params.get("vs_a").unwrap();
    assert_eq!(record.max_num_results, None);
    assert_eq!(record.filters, Some(object(vec![("tool", text("x"))])));
    assert_eq!(ids, vec!["vs_b", "vs_a", "vs_a"]);
    assert!(tools.is_empty());
}

async fn missing_and_empty_tools_change_nothing() {
    let registry = registry(vec![store("vs_a")]);
    let mut empty_tools = Vec::new();
    let mut ids = vec!["vs_a".to_owned()];
    let params = pop_recognised_tools(&registry, &mut empty_tools, &mut ids).unwrap();
    assert!(params.is_empty());
    assert_eq!(ids, vec!["vs_a"]);
    let mut no_tools_registry = copied(&registry);
    let mut params_in = object(vec![("vector_store_ids", list(&["vs_a"]))]);
    let selected = pop_stores_with_fallback(
        &mut no_tools_registry,
        &mut params_in,
        None,
        None::<&ScriptedDatabase>,
    )
    .await
    .unwrap();
    assert_eq!(selected.len(), 1);
    assert_eq!(no_tools_registry.stores, vec![store("vs_a")]);
}

async fn wrong_id_types_name_their_field() {
    tool_id_types_are_errors();
    request_id_types_are_errors().await;
}

fn tool_id_types_are_errors() {
    let registry = registry(vec![store("vs_a")]);
    let mut scalars = vec![object(vec![("vector_store_ids", text("vs_a"))])];
    let mut ids = Vec::new();
    let error = pop_recognised_tools(&registry, &mut scalars, &mut ids).unwrap_err();
    assert_eq!(
        error.to_string(),
        "tool.vector_store_ids must be a list of strings"
    );
    let mut numbers = vec![object(vec![(
        "vector_store_ids",
        Json::List(vec![number(1)]),
    )])];
    let error = pop_recognised_tools(&registry, &mut numbers, &mut Vec::new()).unwrap_err();
    assert_eq!(
        error.to_string(),
        "tool.vector_store_ids must be a list of strings"
    );
}

async fn request_id_types_are_errors() {
    let registry = registry(vec![store("vs_a")]);
    for value in [text("vs_a"), Json::List(vec![number(1)])] {
        let mut working = copied(&registry);
        let mut params = object(vec![("vector_store_ids", value)]);
        let error =
            pop_stores_with_fallback(&mut working, &mut params, None, None::<&ScriptedDatabase>)
                .await
                .unwrap_err();
        assert_eq!(
            error.to_string(),
            "vector_store_ids must be a list of strings"
        );
    }
}

#[tokio::test]
async fn selection_order_and_overrides() {
    stored_params_win_in_request_order().await;
    tool_params_fill_absent_stored_values().await;
    an_empty_tool_sets_an_empty_record().await;
    a_non_object_stored_record_is_an_error().await;
    absent_and_empty_request_ids_start_empty().await;
}

async fn an_empty_tool_sets_an_empty_record() {
    let mut registry = registry(vec![store("vs_a")]);
    let mut params = object(vec![("vector_store_ids", list(&["vs_a"]))]);
    let mut tools = vec![tool("vs_a", vec![])];
    let selected = pop_stores_with_fallback(
        &mut registry,
        &mut params,
        Some(&mut tools),
        None::<&ScriptedDatabase>,
    )
    .await
    .unwrap();
    assert_eq!(
        selected[0].litellm_params,
        Some(serde_json::to_value(Json::Object(JsonObject::new())).unwrap())
    );
    assert_eq!(registry.stores, vec![store("vs_a")]);
}

fn tool_parameter_fields() -> Vec<(&'static str, Json)> {
    vec![
        ("max_num_results", number(2)),
        ("filters", params_json(object(vec![("tool", text("x"))]))),
        (
            "ranking_options",
            params_json(object(vec![("ranker", text("default"))])),
        ),
    ]
}

async fn tool_params_fill_absent_stored_values() {
    let mut registry = registry(vec![
        store("vs_a"),
        with_params(store("vs_b"), object(vec![("api_key", text("k"))])),
    ]);
    let mut params = object(vec![("vector_store_ids", list(&["vs_a", "vs_b"]))]);
    let mut tools = vec![tool("vs_a", tool_parameter_fields())];
    let selected = pop_stores_with_fallback(
        &mut registry,
        &mut params,
        Some(&mut tools),
        None::<&ScriptedDatabase>,
    )
    .await
    .unwrap();
    assert_eq!(
        selected[0].litellm_params,
        Some(serde_json::to_value(params_json(object(tool_parameter_fields()))).unwrap())
    );
    assert_eq!(
        selected[1].litellm_params,
        Some(serde_json::to_value(params_json(object(vec![("api_key", text("k"))]))).unwrap())
    );
    assert!(tools.is_empty());
}

async fn a_non_object_stored_record_is_an_error() {
    let mut record = store("vs_a");
    record.litellm_params = Some(serde_json::to_value(text("bad")).unwrap());
    let mut registry = registry(vec![record]);
    let mut params = object(vec![
        ("model", text("gpt-4o")),
        ("vector_store_ids", list(&["vs_a"])),
    ]);
    let mut tools = vec![tool("vs_a", vec![("max_num_results", number(2))])];
    let error = pop_stores_with_fallback(
        &mut registry,
        &mut params,
        Some(&mut tools),
        None::<&ScriptedDatabase>,
    )
    .await
    .unwrap_err();
    assert!(matches!(error, VectorToolSelectionError::StoredParams));
    assert_eq!(params, object(vec![("model", text("gpt-4o"))]));
    assert!(tools.is_empty());
}

fn ordered_selection_inputs() -> (VectorStoreRegistry, JsonObject, Vec<JsonObject>) {
    let stores = registry(vec![
        with_params(store("vs_a"), stored_params_original()),
        store("vs_b"),
    ]);
    let params = object(vec![
        ("model", text("gpt-4o")),
        (
            "vector_store_ids",
            list(&["vs_b", "vs_a", "vs_b", "unknown"]),
        ),
    ]);
    let tools = vec![tool(
        "vs_a",
        vec![
            ("max_num_results", number(2)),
            ("filters", params_json(object(vec![("tool", text("x"))]))),
        ],
    )];
    (stores, params, tools)
}

async fn stored_params_win_in_request_order() {
    let (mut registry, mut params, mut tools) = ordered_selection_inputs();
    let selected = pop_stores_with_fallback(
        &mut registry,
        &mut params,
        Some(&mut tools),
        None::<&ScriptedDatabase>,
    )
    .await
    .unwrap();
    let ids: Vec<Option<&str>> = selected
        .iter()
        .map(|record| record.vector_store_id.as_deref())
        .collect();
    assert_eq!(
        ids,
        vec![Some("vs_b"), Some("vs_a"), Some("vs_b"), Some("vs_a")]
    );
    assert_eq!(selected[0].litellm_params, None);
    assert_eq!(
        selected[1].litellm_params,
        Some(serde_json::to_value(params_json(stored_params_original())).unwrap())
    );
    assert_eq!(selected[2], selected[0]);
    assert_eq!(selected[3], selected[1]);
    assert_eq!(params, object(vec![("model", text("gpt-4o"))]));
    assert!(tools.is_empty());
    assert_eq!(
        registry.stores[0].litellm_params,
        Some(serde_json::to_value(params_json(stored_params_original())).unwrap())
    );
    assert_eq!(registry.stores[1], store("vs_b"));
}

fn stored_params_original() -> JsonObject {
    object(vec![
        ("max_num_results", number(9)),
        ("filters", params_json(object(vec![("stored", text("y"))]))),
        ("api_key", text("k")),
    ])
}

async fn absent_and_empty_request_ids_start_empty() {
    let registry = registry(vec![store("vs_a")]);
    for params_in in [
        object(vec![]),
        object(vec![("vector_store_ids", Json::Null)]),
        object(vec![("vector_store_ids", list(&[]))]),
    ] {
        let mut params = params_in;
        let mut tools = vec![tool("vs_a", vec![("max_num_results", number(2))])];
        let mut working = copied(&registry);
        let selected = pop_stores_with_fallback(
            &mut working,
            &mut params,
            Some(&mut tools),
            None::<&ScriptedDatabase>,
        )
        .await
        .unwrap();
        assert_eq!(selected.len(), 1);
        assert_eq!(
            selected[0].litellm_params,
            Some(
                serde_json::to_value(params_json(object(vec![("max_num_results", number(2))])))
                    .unwrap()
            )
        );
        assert!(params.is_empty());
        assert!(tools.is_empty());
    }
}

#[test]
fn first_store_and_name_lookup() {
    first_store_follows_the_deduplicated_ids();
    name_lookup_returns_the_first_match();
}

fn named(id: &str, name: &str) -> LiteLlmManagedVectorStore {
    let mut record = store(id);
    record.vector_store_name = Some(name.to_owned());
    record
}

fn first_store_follows_the_deduplicated_ids() {
    let registry = registry(vec![
        named("vs_a", "notes"),
        named("vs_b", "first"),
        store("vs_c"),
    ]);
    let params = object(vec![(
        "vector_store_ids",
        list(&["unknown", "vs_b", "vs_a", "vs_b"]),
    )]);
    let mut tools = vec![tool("vs_a", vec![])];
    let found = get_vector_store_to_run(&registry, &params, Some(tools.as_slice()))
        .unwrap()
        .expect("vs_b answers first");
    assert_eq!(found.vector_store_id.as_deref(), Some("vs_b"));
    assert_eq!(
        params.get("vector_store_ids"),
        Some(&list(&["unknown", "vs_b", "vs_a", "vs_b"]))
    );
    let removed = pop_recognised_tools(&registry, &mut tools, &mut Vec::new()).unwrap();
    assert_eq!(removed.len(), 1);
}

fn name_lookup_returns_the_first_match() {
    let registry = registry(vec![
        named("vs_a", "notes"),
        named("vs_b", "notes"),
        store("vs_c"),
    ]);
    let found = get_vector_store_by_name(&registry, "notes").unwrap();
    assert_eq!(found.vector_store_id.as_deref(), Some("vs_a"));
    assert_eq!(get_vector_store_by_name(&registry, "missing"), None);
    assert_eq!(registry.stores.len(), 3);
}

#[tokio::test]
async fn cache_verification() {
    config_owned_entries_skip_the_database().await;
    a_database_row_keeps_the_cached_record().await;
    a_deleted_entry_is_evicted_and_refetched().await;
    a_verification_error_keeps_the_stale_record().await;
    a_fallback_adds_the_matching_row().await;
    a_fallback_without_the_row_skips_the_id().await;
    a_fallback_error_skips_the_id().await;
    no_database_uses_memory_only().await;
    duplicate_ids_verify_twice().await;
}

async fn a_fallback_adds_the_matching_row() {
    let mut registry = registry(vec![]);
    let mut row = store("vs_a");
    row.custom_llm_provider = Some("bedrock".to_owned());
    let database = ScriptedDatabase::with_rows(vec![store("vs_b"), row.clone()]);
    let selected = pop_ids(&mut registry, &["vs_a"], Some(&database)).await;
    assert_eq!(selected, vec![row.clone()]);
    assert_eq!(registry.stores, vec![row]);
    assert_eq!(database.calls(), vec!["find_many"]);
}

async fn a_fallback_without_the_row_skips_the_id() {
    let mut registry = registry(vec![store("vs_a")]);
    let database = ScriptedDatabase::with_rows(vec![store("vs_b")]);
    let selected = pop_ids(&mut registry, &["vs_a"], Some(&database)).await;
    assert!(selected.is_empty());
    assert!(registry.stores.is_empty());
    assert_eq!(database.calls(), vec!["find_unique:vs_a", "find_many"]);
}

async fn config_owned_entries_skip_the_database() {
    let mut config = store("vs_a");
    config.is_config = Some(true);
    let mut registry = registry(vec![config.clone()]);
    let database = ScriptedDatabase::without_rows()
        .with_verification_failure()
        .with_read_failure();
    let (logs, guard) = CapturedLogs::install();
    let selected = pop_ids(&mut registry, &["vs_a"], Some(&database)).await;
    drop(guard);
    assert_eq!(selected, vec![config]);
    assert!(database.calls().is_empty());
    assert!(logs.text().is_empty());
}

async fn a_database_row_keeps_the_cached_record() {
    let mut cached = store("vs_a");
    cached.vector_store_description = Some("memory".to_owned());
    let mut row = store("vs_a");
    row.vector_store_description = Some("from-database".to_owned());
    row.custom_llm_provider = Some("bedrock".to_owned());
    let mut registry = registry(vec![cached.clone()]);
    let database = ScriptedDatabase::with_rows(vec![row.clone()]).with_verifiable(row);
    let selected = pop_ids(&mut registry, &["vs_a"], Some(&database)).await;
    assert_eq!(selected, vec![cached.clone()]);
    assert_eq!(registry.stores, vec![cached]);
    assert_eq!(database.calls(), vec!["find_unique:vs_a"]);
}

async fn a_deleted_entry_is_evicted_and_refetched() {
    let mut cached = store("vs_a");
    cached.is_config = Some(false);
    let mut row = store("vs_a");
    row.vector_store_description = Some("from-database".to_owned());
    let mut registry = registry(vec![cached]);
    let database = ScriptedDatabase::with_rows(vec![row.clone()]);
    let (logs, guard) = CapturedLogs::install();
    let selected = pop_ids(&mut registry, &["vs_a"], Some(&database)).await;
    drop(guard);
    assert_eq!(selected, vec![row.clone()]);
    assert_eq!(registry.stores, vec![row]);
    assert_eq!(database.calls(), vec!["find_unique:vs_a", "find_many"]);
    assert!(logs.text().contains(
        "Vector store vs_a found in memory but deleted from database, removing from cache"
    ));
}

async fn a_verification_error_keeps_the_stale_record() {
    let cached = store("vs_a");
    let mut row = store("vs_a");
    row.vector_store_description = Some("from-database".to_owned());
    let mut registry = registry(vec![cached.clone()]);
    let database = ScriptedDatabase::with_rows(vec![row]).with_verification_failure();
    let (logs, guard) = CapturedLogs::install();
    let selected = pop_ids(&mut registry, &["vs_a"], Some(&database)).await;
    drop(guard);
    assert_eq!(selected, vec![cached]);
    assert_eq!(database.calls(), vec!["find_unique:vs_a"]);
    assert!(
        logs.text()
            .contains("Error verifying vector store vs_a in database:")
    );
}

async fn a_fallback_error_skips_the_id() {
    let mut registry = registry(vec![]);
    let database = ScriptedDatabase::without_rows().with_read_failure();
    let (logs, guard) = CapturedLogs::install();
    let selected = pop_ids(&mut registry, &["vs_a"], Some(&database)).await;
    drop(guard);
    assert!(selected.is_empty());
    assert_eq!(database.calls(), vec!["find_many"]);
    assert!(
        logs.text()
            .contains("Error fetching vector store vs_a from database:")
    );
}

async fn no_database_uses_memory_only() {
    let mut registry = registry(vec![store("vs_a")]);
    let selected = pop_ids(&mut registry, &["vs_a", "vs_missing"], None).await;
    assert_eq!(selected, vec![store("vs_a")]);
}

async fn duplicate_ids_verify_twice() {
    let cached = store("vs_a");
    let mut registry = registry(vec![cached.clone()]);
    let database = ScriptedDatabase::without_rows().with_verifiable(cached.clone());
    let selected = pop_ids(&mut registry, &["vs_a", "vs_a"], Some(&database)).await;
    assert_eq!(selected, vec![cached.clone(), cached]);
    assert_eq!(
        database.calls(),
        vec!["find_unique:vs_a", "find_unique:vs_a"]
    );
}

async fn pop_ids(
    registry: &mut VectorStoreRegistry,
    ids: &[&str],
    database: Option<&ScriptedDatabase>,
) -> Vec<LiteLlmManagedVectorStore> {
    let mut params = object(vec![("vector_store_ids", list(ids))]);
    pop_stores_with_fallback(registry, &mut params, None, database)
        .await
        .unwrap()
}

#[tokio::test]
async fn tool_parameter_failure() {
    a_failing_tool_keeps_its_deferred_removal();
    the_consuming_path_keeps_its_mutations().await;
}

fn a_failing_tool_keeps_its_deferred_removal() {
    let registry = registry(vec![store("vs_a")]);
    let mut tools = vec![tool("vs_a", vec![("rewrite_query", Json::Bool(false))])];
    let mut ids = vec!["vs_b".to_owned()];
    let error = pop_recognised_tools(&registry, &mut tools, &mut ids).unwrap_err();
    assert!(matches!(
        error,
        VectorToolSelectionError::Registry(VectorStoreRegistryError::RewriteQueryNotSupported)
    ));
    assert_eq!(ids, vec!["vs_b", "vs_a"]);
    assert_eq!(tools.len(), 1);
}

async fn the_consuming_path_keeps_its_mutations() {
    let mut registry = registry(vec![store("vs_a")]);
    let mut params = object(vec![
        ("model", text("gpt-4o")),
        ("vector_store_ids", list(&["vs_b"])),
    ]);
    let mut tools = vec![tool("vs_a", vec![("rewrite_query", Json::Bool(false))])];
    let error = pop_stores_with_fallback(
        &mut registry,
        &mut params,
        Some(&mut tools),
        None::<&ScriptedDatabase>,
    )
    .await
    .unwrap_err();
    assert!(matches!(
        error,
        VectorToolSelectionError::Registry(VectorStoreRegistryError::RewriteQueryNotSupported)
    ));
    assert_eq!(params, object(vec![("model", text("gpt-4o"))]));
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].get("rewrite_query"), Some(&Json::Bool(false)));
    assert_eq!(registry.stores, vec![store("vs_a")]);
}
