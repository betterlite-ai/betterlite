#![cfg(test)]

mod postgres;
mod support;

use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_auth::auth_model_lists::credential::{CredentialEntry, CredentialSnapshot};
use betterlite_config::api::vector_store_registry::RegistryEntry;
use betterlite_db::auxiliary_repository::repository::TableError;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_openai::api::vector_store_records::records::pydantic::LiteLlmManagedVectorStoreIndex;
use betterlite_openai::api::vector_store_records::records::vector::LiteLlmManagedVectorStore;
use betterlite_redact::structured::REDACTED;
use chrono::SecondsFormat;

use support::{
    FIXED_TIMESTAMP, ScriptedDatabase, capture_logs, config_store, database_store, encoded,
    fixed_clock, index, inline_key_store, labelled_store, named_store, registry,
    registry_with_credentials, store,
};

use super::error::VectorStoreRegistryError;
use super::index::VectorStoreIndexRegistry;
use super::repository::rows_from_database;
use super::store::{VectorStoreCredentials, VectorStoreRegistry};
use super::tools::{VectorStoreToolParams, extract_tool_params, ids_to_run};

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn params(pairs: &[(&str, Json)]) -> JsonObject {
    let mut fields = JsonObject::new();
    for (name, value) in pairs {
        fields.insert((*name).to_owned(), value.clone());
    }
    fields
}

fn entry(name: Option<&str>, fields: Option<JsonObject>) -> RegistryEntry {
    RegistryEntry {
        vector_store_name: name.map(str::to_owned),
        litellm_params: fields,
    }
}

fn ids(registry: &VectorStoreRegistry) -> Vec<Option<&str>> {
    let mut ids = Vec::new();
    for store in &registry.stores {
        ids.push(store.vector_store_id.as_deref());
    }
    ids
}

fn required(params: &mut JsonObject) {
    params.insert("vector_store_id".to_owned(), text("vs_a"));
    params.insert("custom_llm_provider".to_owned(), text("openai"));
}

#[test]
fn registry_config_records() {
    config_records_keep_every_field();
    config_records_accept_missing_names_and_empty_values();
    config_records_reject_missing_ids();
    config_records_reject_missing_providers();
    config_records_append_duplicates();
    config_records_keep_the_prefix_before_a_failure();
}

fn config_records_keep_every_field() {
    let full = params(&[
        ("vector_store_id", text("vs_a")),
        ("custom_llm_provider", text("openai")),
        ("vector_store_description", text("d")),
        (
            "vector_store_metadata",
            Json::Object(params(&[("tag", text("x"))])),
        ),
    ]);
    let mut loaded = registry(Vec::new());
    loaded
        .load_vector_stores_from_config(&[entry(Some("notes"), Some(full))])
        .unwrap();
    let store = loaded.stores.first().unwrap();
    assert_eq!(store.vector_store_id.as_deref(), Some("vs_a"));
    assert_eq!(store.custom_llm_provider.as_deref(), Some("openai"));
    assert_eq!(store.vector_store_name.as_deref(), Some("notes"));
    assert_eq!(store.vector_store_description.as_deref(), Some("d"));
    assert_config_record_json(store);
    assert_eq!(store.created_at.as_deref(), Some(FIXED_TIMESTAMP));
    assert_eq!(store.updated_at.as_deref(), Some(FIXED_TIMESTAMP));
    assert_eq!(store.is_config, Some(true));

    let mut raw_metadata = JsonObject::new();
    required(&mut raw_metadata);
    raw_metadata.insert("vector_store_metadata".to_owned(), text("raw"));
    let mut string_metadata = registry(Vec::new());
    string_metadata
        .load_vector_stores_from_config(&[entry(None, Some(raw_metadata))])
        .unwrap();
    assert_eq!(
        string_metadata
            .stores
            .first()
            .unwrap()
            .vector_store_metadata
            .as_ref()
            .map(encoded)
            .as_deref(),
        Some("\"raw\""),
        "a string metadata is kept as the declared dict | str | None union"
    );
}

fn assert_config_record_json(store: &LiteLlmManagedVectorStore) {
    assert_eq!(
        store.vector_store_metadata.as_ref().map(encoded).as_deref(),
        Some(r#"{"tag":"x"}"#)
    );
    assert_eq!(
        serde_json::from_str::<JsonObject>(
            &store.litellm_params.as_ref().map(encoded).expect("params")
        )
        .unwrap(),
        params(&[
            ("vector_store_id", text("vs_a")),
            ("custom_llm_provider", text("openai")),
            ("vector_store_description", text("d")),
            (
                "vector_store_metadata",
                Json::Object(params(&[("tag", text("x"))])),
            ),
        ])
    );
}

fn config_records_accept_missing_names_and_empty_values() {
    let mut nameless = registry(Vec::new());
    let mut only_required = JsonObject::new();
    required(&mut only_required);
    nameless
        .load_vector_stores_from_config(&[entry(None, Some(only_required))])
        .unwrap();
    assert_eq!(nameless.stores.first().unwrap().vector_store_name, None);

    let mut empty = registry(Vec::new());
    let empty_params = params(&[
        ("vector_store_id", text("")),
        ("custom_llm_provider", text("")),
    ]);
    empty
        .load_vector_stores_from_config(&[entry(Some(""), Some(empty_params))])
        .unwrap();
    let accepted = empty.stores.first().unwrap();
    assert_eq!(accepted.vector_store_id.as_deref(), Some(""));
    assert_eq!(accepted.custom_llm_provider.as_deref(), Some(""));
}

fn config_records_reject_missing_ids() {
    let mut null_params = registry(Vec::new());
    let null_params_error = null_params
        .load_vector_stores_from_config(&[entry(Some("notes"), None)])
        .unwrap_err();
    assert_eq!(
        null_params_error.to_string(),
        "vector_store_id is required for initializing vector store, got vector_store_id=None"
    );
    assert!(matches!(
        null_params_error,
        VectorStoreRegistryError::MissingVectorStoreId
    ));

    let mut missing_id = JsonObject::new();
    missing_id.insert("custom_llm_provider".to_owned(), text("openai"));
    let mut registry_missing_id = registry(Vec::new());
    let missing_id_error = registry_missing_id
        .load_vector_stores_from_config(&[entry(None, Some(missing_id))])
        .unwrap_err();
    assert!(matches!(
        missing_id_error,
        VectorStoreRegistryError::MissingVectorStoreId
    ));

    let mut null_id = JsonObject::new();
    null_id.insert("vector_store_id".to_owned(), Json::Null);
    null_id.insert("custom_llm_provider".to_owned(), text("openai"));
    let mut registry_null_id = registry(Vec::new());
    let null_id_error = registry_null_id
        .load_vector_stores_from_config(&[entry(None, Some(null_id))])
        .unwrap_err();
    assert!(matches!(
        null_id_error,
        VectorStoreRegistryError::MissingVectorStoreId
    ));
}

fn config_records_reject_missing_providers() {
    let mut missing_provider = JsonObject::new();
    missing_provider.insert("vector_store_id".to_owned(), text("vs_a"));
    let mut registry_missing_provider = registry(Vec::new());
    let missing_provider_error = registry_missing_provider
        .load_vector_stores_from_config(&[entry(None, Some(missing_provider.clone()))])
        .unwrap_err();
    assert_eq!(
        missing_provider_error.to_string(),
        "custom_llm_provider is required for initializing vector store, got custom_llm_provider=None"
    );
    assert!(matches!(
        missing_provider_error,
        VectorStoreRegistryError::MissingCustomLlmProvider
    ));

    let mut null_provider = missing_provider;
    null_provider.insert("custom_llm_provider".to_owned(), Json::Null);
    let mut registry_null_provider = registry(Vec::new());
    let null_provider_error = registry_null_provider
        .load_vector_stores_from_config(&[entry(None, Some(null_provider))])
        .unwrap_err();
    assert!(matches!(
        null_provider_error,
        VectorStoreRegistryError::MissingCustomLlmProvider
    ));
}

fn config_records_append_duplicates() {
    let mut duplicates = registry(Vec::new());
    let mut duplicate_params = JsonObject::new();
    required(&mut duplicate_params);
    duplicates
        .load_vector_stores_from_config(&[
            entry(Some("first"), Some(duplicate_params.clone())),
            entry(Some("second"), Some(duplicate_params)),
        ])
        .unwrap();
    assert_eq!(
        ids(&duplicates),
        vec![Some("vs_a"), Some("vs_a")],
        "duplicate config ids append twice"
    );
}

fn config_records_keep_the_prefix_before_a_failure() {
    let mut partial = registry(Vec::new());
    let mut valid = JsonObject::new();
    required(&mut valid);
    let (logs, _guard) = capture_logs();
    let partial_error = partial
        .load_vector_stores_from_config(&[
            entry(Some("first"), Some(valid)),
            entry(Some("broken"), None),
        ])
        .unwrap_err();
    assert!(matches!(
        partial_error,
        VectorStoreRegistryError::MissingVectorStoreId
    ));
    assert_eq!(
        ids(&partial),
        vec![Some("vs_a")],
        "the earlier entry stays appended"
    );
    assert!(
        logs.debug().is_empty(),
        "the load diagnostic comes after the whole loop"
    );
}

#[test]
fn ordered_registry_mutations() {
    mutation_order_is_kept();
    config_owned_mutations_are_refused();
    missing_id_updates_append();
    config_ownership_and_deletion();
}

fn mutation_order_is_kept() {
    let mut registry = registry(vec![store("vs_a"), store("vs_b")]);
    registry.add_store(labelled_store("vs_a", "ignored"));
    assert_eq!(ids(&registry), vec![Some("vs_a"), Some("vs_b")]);
    assert_eq!(
        registry
            .store_by_id("vs_a")
            .unwrap()
            .vector_store_description
            .as_deref(),
        Some("listed"),
        "an id already present keeps the first record"
    );
    registry.add_store(store("vs_c"));
    assert_eq!(
        ids(&registry),
        vec![Some("vs_a"), Some("vs_b"), Some("vs_c")]
    );

    registry.update_store("vs_b", labelled_store("vs_b", "replaced"));
    let replaced = registry.store_by_id("vs_b").unwrap();
    assert_eq!(
        replaced.vector_store_description.as_deref(),
        Some("replaced"),
        "the whole record is replaced rather than merged"
    );
    assert_eq!(
        replaced.litellm_credential_name.as_deref(),
        Some("stored-credential")
    );
    registry.update_store("vs_a", labelled_store("vs_a", "moved"));
    assert_eq!(
        ids(&registry),
        vec![Some("vs_a"), Some("vs_b"), Some("vs_c")],
        "an update keeps the record where it is"
    );
}

fn config_owned_mutations_are_refused() {
    let mut registry = registry(Vec::new());
    registry.add_store(config_store("vs_cfg"));
    registry.update_store("vs_cfg", labelled_store("vs_cfg", "refused"));
    let config = registry.store_by_id("vs_cfg").unwrap();
    assert_eq!(config.is_config, Some(true));
    assert_eq!(
        config.vector_store_description, None,
        "a config-owned record is left untouched"
    );
}

fn missing_id_updates_append() {
    let mut registry = registry(Vec::new());
    registry.update_store("vs_missing", store("vs_other"));
    assert_eq!(
        registry.stores.last().unwrap().vector_store_id.as_deref(),
        Some("vs_other"),
        "an update for a missing id appends the update it carries"
    );
}

fn config_ownership_and_deletion() {
    let registry = registry(vec![store("vs_a"), config_store("vs_b")]);
    assert!(!registry.is_config("vs_a"));
    assert!(registry.is_config("vs_b"));
    assert!(!registry.is_config("vs_missing"));

    let mut owned = support::registry(vec![store("vs_a"), config_store("vs_a"), store("vs_b")]);
    owned.remove_store("vs_a");
    assert_eq!(
        ids(&owned),
        vec![Some("vs_b")],
        "deletion takes every matching record, config-owned included"
    );
    assert!(owned.store_by_id("vs_a").is_none());
}

#[test]
fn index_registry_order_and_lookup() {
    let mut registry = VectorStoreIndexRegistry {
        indexes: vec![index("i1", "notes"), index("i2", "notes")],
    };
    assert_eq!(registry.find_by_name("notes").unwrap().id, "i1");
    assert!(registry.contains_name("notes"));
    assert!(!registry.contains_name("absent"));
    assert!(registry.find_by_name("absent").is_none());
    registry.upsert(index("i3", "notes"));
    assert_eq!(registry.indexes.len(), 2);
    assert_eq!(
        registry.find_by_name("notes").unwrap().id,
        "i3",
        "upsert replaces the first matching record in place"
    );
    assert_eq!(registry.indexes.get(1).expect("the second index").id, "i2");
    registry.upsert(index("i4", "archive"));
    assert_eq!(registry.indexes.len(), 3);
    assert_eq!(registry.find_by_name("archive").unwrap().id, "i4");
    assert!(VectorStoreIndexRegistry::default().indexes.is_empty());
    assert!(VectorStoreRegistry::default().stores.is_empty());
}

#[tokio::test]
async fn registry_database_fallback() {
    memory_hit_never_reaches_the_database().await;
    database_hit_joins_and_caches_the_registry().await;
    database_failure_is_logged_and_loads_nothing().await;
    missing_database_reads_no_rows().await;
    store_rows_keep_the_database_order().await;
    index_rows_keep_the_database_order().await;
    row_read_errors_keep_their_table_error().await;
}

async fn memory_hit_never_reaches_the_database() {
    let database = ScriptedDatabase::with_stores(vec![database_store("vs_db")]);
    let mut registry = registry(vec![store("vs_a")]);
    let remembered = registry
        .lookup_or_database("vs_a", Some(&database))
        .await
        .unwrap();
    assert_eq!(remembered.vector_store_id.as_deref(), Some("vs_a"));
    assert!(
        database.calls().is_empty(),
        "a memory hit never reaches the database"
    );

    assert!(
        registry
            .lookup_or_database("vs_db", None::<&ScriptedDatabase>)
            .await
            .is_none()
    );
    assert_eq!(ids(&registry), vec![Some("vs_a")]);
}

async fn database_hit_joins_and_caches_the_registry() {
    let database = ScriptedDatabase::with_stores(vec![database_store("vs_db")]);
    let mut registry = registry(vec![store("vs_a")]);
    let found = registry
        .lookup_or_database("vs_db", Some(&database))
        .await
        .unwrap();
    assert_eq!(found.vector_store_id.as_deref(), Some("vs_db"));
    assert_eq!(found.custom_llm_provider.as_deref(), Some("bedrock"));
    assert_eq!(database.calls(), vec!["stores"]);
    assert!(
        registry.store_by_id("vs_db").is_some(),
        "a database hit joins the registry"
    );
    registry
        .lookup_or_database("vs_db", Some(&database))
        .await
        .unwrap();
    assert_eq!(
        database.calls(),
        vec!["stores"],
        "the cached record answers the next lookup"
    );

    assert!(
        registry
            .lookup_or_database("vs_absent", Some(&database))
            .await
            .is_none()
    );
    assert_eq!(database.calls(), vec!["stores", "stores"]);
}

async fn database_failure_is_logged_and_loads_nothing() {
    let failing = ScriptedDatabase::failing();
    let mut registry = registry(vec![store("vs_a")]);
    let (logs, _guard) = capture_logs();
    assert!(
        registry
            .lookup_or_database("vs_db", Some(&failing))
            .await
            .is_none()
    );
    assert_eq!(failing.calls(), vec!["stores"]);
    assert!(
        logs.debug()
            .iter()
            .any(|message| message.contains("Error fetching vector store from database:")),
        "a database failure is logged at debug level"
    );
}

async fn missing_database_reads_no_rows() {
    assert!(
        rows_from_database::<LiteLlmManagedVectorStore, ScriptedDatabase>(None)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        rows_from_database::<LiteLlmManagedVectorStoreIndex, ScriptedDatabase>(None)
            .await
            .unwrap()
            .is_empty()
    );
}

async fn store_rows_keep_the_database_order() {
    let rows =
        ScriptedDatabase::with_stores(vec![database_store("vs_new"), database_store("vs_old")]);
    let loaded_stores = rows_from_database::<LiteLlmManagedVectorStore, _>(Some(&rows))
        .await
        .unwrap();
    assert_eq!(loaded_stores.len(), 2);
    assert_eq!(
        loaded_stores.first().unwrap().vector_store_id.as_deref(),
        Some("vs_new")
    );
    assert_eq!(
        loaded_stores
            .get(1)
            .expect("the second store row")
            .vector_store_id
            .as_deref(),
        Some("vs_old")
    );
}

async fn index_rows_keep_the_database_order() {
    let scripted_indexes =
        ScriptedDatabase::with_indexes(vec![index("i1", "notes"), index("i2", "archive")]);
    let loaded_indexes =
        rows_from_database::<LiteLlmManagedVectorStoreIndex, _>(Some(&scripted_indexes))
            .await
            .unwrap();
    assert_eq!(loaded_indexes.first().unwrap().index_name, "notes");
    assert_eq!(
        loaded_indexes
            .get(1)
            .expect("the second index row")
            .index_name,
        "archive"
    );
}

async fn row_read_errors_keep_their_table_error() {
    let failing = ScriptedDatabase::failing();
    let stores_error = rows_from_database::<LiteLlmManagedVectorStore, _>(Some(&failing))
        .await
        .unwrap_err();
    assert!(matches!(stores_error, TableError::Publish { .. }));
    let indexes_error = rows_from_database::<LiteLlmManagedVectorStoreIndex, _>(Some(&failing))
        .await
        .unwrap_err();
    assert!(matches!(indexes_error, TableError::Publish { .. }));
}

#[test]
fn registry_id_and_tool_params() {
    vector_store_ids_collect_in_order();
    tool_params_extract_declared_fields();
    rewrite_query_is_rejected();
}

fn vector_store_ids_collect_in_order() {
    let non_default = params(&[(
        "vector_store_ids",
        Json::List(vec![text("vs_b"), text("vs_a"), text("vs_b")]),
    )]);
    let tools = vec![
        params(&[("type", text("function")), ("name", text("weather"))]),
        params(&[
            ("type", text("file_search")),
            (
                "vector_store_ids",
                Json::List(vec![text("vs_c"), text("vs_a")]),
            ),
        ]),
    ];
    let before = tools.clone();
    let collected = ids_to_run(&non_default, Some(&tools)).unwrap();
    assert_eq!(
        collected,
        vec!["vs_b", "vs_a", "vs_c"],
        "a tool without a vector_store_ids key is skipped, not rejected"
    );
    assert_eq!(tools, before, "the tools are left as sent");

    let scalar = params(&[("vector_store_ids", text("vs_a"))]);
    assert!(ids_to_run(&scalar, None).unwrap().is_empty());
    let null_ids = params(&[("vector_store_ids", Json::Null)]);
    assert!(ids_to_run(&null_ids, None).unwrap().is_empty());
    let empty = params(&[("vector_store_ids", Json::List(Vec::new()))]);
    assert!(ids_to_run(&empty, None).unwrap().is_empty());
    let other_type = vec![params(&[
        ("type", text("other")),
        ("vector_store_ids", Json::List(vec![text("vs_x")])),
    ])];
    assert_eq!(
        ids_to_run(&params(&[]), Some(&other_type)).unwrap(),
        vec!["vs_x"],
        "the tool type is never consulted"
    );
}

fn tool_params_extract_declared_fields() {
    let tool = params(&[
        ("type", text("file_search")),
        ("vector_store_ids", Json::List(vec![text("vs_c")])),
        ("filters", Json::Object(params(&[("kind", text("doc"))]))),
        (
            "max_num_results",
            Json::Number(serde_json::Number::from(0_u8)),
        ),
        ("ranking_options", Json::Object(JsonObject::new())),
    ]);
    let extracted = extract_tool_params(&tool).unwrap();
    assert_eq!(extracted.filters, Some(params(&[("kind", text("doc"))])));
    assert_eq!(extracted.max_num_results, Some(0));
    assert_eq!(
        extracted.ranking_options,
        Some(JsonObject::new()),
        "type and vector_store_ids are ignored, since only the three declared params are read"
    );
    let sized = extract_tool_params(&params(&[(
        "max_num_results",
        Json::Number(serde_json::Number::from(25_u8)),
    )]))
    .unwrap();
    assert_eq!(sized.max_num_results, Some(25));

    let absent = extract_tool_params(&params(&[])).unwrap();
    assert_eq!(absent, VectorStoreToolParams::default());
    assert_eq!(absent.filters, None);
    assert_eq!(absent.max_num_results, None);
    assert_eq!(absent.ranking_options, None);
    let nulls = extract_tool_params(&params(&[
        ("filters", Json::Null),
        ("max_num_results", Json::Null),
        ("ranking_options", Json::Null),
    ]))
    .unwrap();
    assert_eq!(nulls, VectorStoreToolParams::default());
}

fn rewrite_query_is_rejected() {
    let rewrite = params(&[("rewrite_query", Json::Bool(false))]);
    let error = extract_tool_params(&rewrite).unwrap_err();
    assert_eq!(
        error.to_string(),
        "rewrite_query is not a supported tool parameter"
    );
    assert!(matches!(
        error,
        VectorStoreRegistryError::RewriteQueryNotSupported
    ));
}

#[test]
fn registry_named_credentials() {
    named_credentials_read_the_accessor();
    named_credentials_skip_empty_and_inline_names();
    named_credentials_answer_the_first_record_with_a_name();
}

fn named_credentials_read_the_accessor() {
    let accessor = Arc::new(support::RecordingCredentials::new(BTreeMap::from([(
        "cred".to_owned(),
        params(&[("api_key", text("secret-value"))]),
    )])));
    let credentials: Arc<dyn VectorStoreCredentials> = accessor.clone();
    let registry = registry_with_credentials(vec![named_store("vs_cred", "cred")], credentials);
    assert_eq!(
        registry.credentials_for_vector_store("vs_cred"),
        params(&[("api_key", text("secret-value"))])
    );
    assert_eq!(accessor.names(), vec!["cred"]);

    assert!(
        registry
            .credentials_for_vector_store("vs_absent")
            .is_empty()
    );
    assert_eq!(
        accessor.names(),
        vec!["cred"],
        "an unknown id never reaches the accessor"
    );
}

fn named_credentials_skip_empty_and_inline_names() {
    let empty_accessor = Arc::new(support::RecordingCredentials::new(BTreeMap::new()));
    let empty_credentials: Arc<dyn VectorStoreCredentials> = empty_accessor.clone();
    let empty_registry = registry_with_credentials(
        vec![
            named_store("vs_empty", ""),
            inline_key_store("vs_inline"),
            named_store("vs_dup", "absent"),
        ],
        empty_credentials,
    );
    assert!(
        empty_registry
            .credentials_for_vector_store("vs_empty")
            .is_empty()
    );
    assert!(
        empty_registry
            .credentials_for_vector_store("vs_inline")
            .is_empty()
    );
    assert_eq!(
        empty_accessor.names(),
        Vec::<String>::new(),
        "an empty name and an inline key skip the accessor"
    );
    assert!(
        empty_registry
            .credentials_for_vector_store("vs_dup")
            .is_empty()
    );
    assert_eq!(
        empty_accessor.names(),
        vec!["absent"],
        "a missing named credential reads the accessor and answers empty"
    );
}

fn named_credentials_answer_the_first_record_with_a_name() {
    let second_accessor = Arc::new(support::RecordingCredentials::new(BTreeMap::from([(
        "second".to_owned(),
        params(&[("api_key", text("second-value"))]),
    )])));
    let second_credentials: Arc<dyn VectorStoreCredentials> = second_accessor;
    let duplicate_registry = registry_with_credentials(
        vec![named_store("vs_dup", ""), named_store("vs_dup", "second")],
        second_credentials,
    );
    assert_eq!(
        duplicate_registry.credentials_for_vector_store("vs_dup"),
        params(&[("api_key", text("second-value"))]),
        "the first record carrying a credential name answers, so a nameless duplicate does not"
    );
}

#[test]
fn credential_snapshot_answers_named_lookups() {
    let snapshot = CredentialSnapshot::new(vec![CredentialEntry {
        credential_name: "cred".to_owned(),
        credential_values: params(&[("api_key", text("snapshot-value"))]),
    }]);
    let registry =
        registry_with_credentials(vec![named_store("vs_cred", "cred")], Arc::new(snapshot));
    assert_eq!(
        registry.credentials_for_vector_store("vs_cred"),
        params(&[("api_key", text("snapshot-value"))]),
        "the production accessor reads the loaded credential snapshot"
    );
    assert!(
        registry
            .credentials_for_vector_store("vs_absent")
            .is_empty()
    );
}

#[test]
fn registry_debug_redacts_credentials() {
    let secret = "sk-registry-secret";
    let document = format!(
        r#"{{"vector_store_id": "vs_secret", "custom_llm_provider": "openai", "litellm_credential_name": "cred", "litellm_params": {{"api_key": "{secret}"}}, "vector_store_metadata": {{"nested": {{"client_password": "{secret}"}}}}}}"#
    );
    let documented: LiteLlmManagedVectorStore = serde_json::from_str(&document).unwrap();
    let registry = registry(vec![documented]);
    let printed = format!("{registry:?}");
    assert!(printed.contains("vs_secret"));
    assert!(printed.contains(REDACTED));
    assert!(!printed.contains(secret));
}

#[test]
fn config_load_diagnostic_redacts_credentials() {
    let secret = "sk-config-secret";
    let mut fields = JsonObject::new();
    required(&mut fields);
    fields.insert("api_key".to_owned(), text(secret));
    let (logs, _guard) = capture_logs();
    let mut registry = registry(Vec::new());
    registry
        .load_vector_stores_from_config(&[entry(Some("notes"), Some(fields))])
        .unwrap();
    let debug = logs.debug();
    assert!(
        debug
            .iter()
            .any(|message| message.contains("all loaded vector stores ="))
    );
    assert!(!debug.iter().any(|message| message.contains(secret)));
    assert!(debug.iter().any(|message| message.contains(REDACTED)));
}

#[test]
fn fixed_clock_matches_the_record_stamp() {
    let clock = fixed_clock();
    assert_eq!(
        clock().to_rfc3339_opts(SecondsFormat::Micros, true),
        FIXED_TIMESTAMP
    );
}
