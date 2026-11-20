use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use axum::http::StatusCode;
use betterlite_auth::auth_pubsub::publish::PublishTasks;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_logging::private_json::error::PrivateJsonError;
use betterlite_mcp::server_record::MCPServer;
use rstest::rstest;
use uuid::Uuid;

use super::attach::attach_object_permission_to_dict;
use super::cache::invalidate_cached_object_permissions;
use super::decode_object_permission;
use super::entries::mcp_tool_permission_entries;
use super::error::ObjectPermissionError;
use super::fixtures::database::TestDatabase;
use super::fixtures::fakes::RecordingEndpoint;
use super::fixtures::fakes::RecordingEviction;
use super::fixtures::fakes::wait_for_publishes;
use super::prepare::non_null_fields;
use super::prepare::prepare_object_permission_upsert;
use super::update::handle_update_object_permission_common;
use super::update::object_permission_id_of;
use super::update::set_object_permission;
use crate::key_fields::error::DecodeError;

fn json(text: &str) -> JsonData {
    serde_json::from_str(text).expect("the case value is JSON")
}

fn fields(text: &str) -> BTreeMap<String, JsonData> {
    let JsonData::Object(fields) = json(text) else {
        panic!("the case value is a JSON object");
    };
    fields
}

fn string(text: &str) -> JsonData {
    JsonData::String(text.to_owned())
}

fn registry_server(text: &str) -> MCPServer {
    serde_json::from_str(text).expect("the registry record is an MCP server")
}

fn collision_message(collisions: &str) -> String {
    format!(
        "Ambiguous mcp_tool_permissions key: {collisions}. Key tool permissions by server_id when \
         servers share a name or alias."
    )
}

fn assert_generated_id(id: &str) {
    assert_eq!(Uuid::parse_str(id).unwrap().to_string(), id);
}

fn permission_input(permission: &str) -> BTreeMap<String, JsonData> {
    let mut data = BTreeMap::new();
    data.insert(
        "object_permission".to_owned(),
        JsonData::Object(fields(permission)),
    );
    data
}

#[tokio::test]
async fn attach_db_first() {
    let mut data = fields(r#"{"object_permission_id": "perm-1"}"#);
    let error = attach_object_permission_to_dict(&mut data, None)
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), "Prisma client not found");
    assert_eq!(data, fields(r#"{"object_permission_id": "perm-1"}"#));

    let mut empty = BTreeMap::new();
    let error = attach_object_permission_to_dict(&mut empty, None)
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), "Prisma client not found");

    let database = TestDatabase::start().await;
    let mut without_id = fields(r#"{"team_id": "t"}"#);
    attach_object_permission_to_dict(&mut without_id, Some(database.prisma()))
        .await
        .unwrap();
    assert_eq!(without_id, fields(r#"{"team_id": "t"}"#));
    database.close().await;
}

#[rstest]
#[case::empty_id(r#"{"object_permission_id": ""}"#)]
#[case::null_id(r#"{"object_permission_id": null}"#)]
#[case::missing_id(r#"{"team_id": "t"}"#)]
#[tokio::test]
async fn attach_falsey_or_missing_id(#[case] case: &str) {
    let database = TestDatabase::start().await;
    let mut data = fields(case);
    attach_object_permission_to_dict(&mut data, Some(database.prisma()))
        .await
        .unwrap();
    assert_eq!(data, fields(case));
    assert!(!data.contains_key("object_permission"));
    assert_eq!(database.permission_count().await, 0);
    database.close().await;
}

#[tokio::test]
async fn attach_v1_fallback() {
    let database = TestDatabase::start().await;
    database
        .seed_permission("perm-1", r#"["srv-a"]"#, Some(r#"{"srv-a": ["t1"]}"#))
        .await;
    let mut found =
        fields(r#"{"object_permission_id": "perm-1", "object_permission": {"old": true}}"#);
    attach_object_permission_to_dict(&mut found, Some(database.prisma()))
        .await
        .unwrap();
    let JsonData::Object(attached) = found.get("object_permission").unwrap() else {
        panic!("the attached row dump is an object");
    };
    assert_eq!(
        attached.get("object_permission_id"),
        Some(&string("perm-1"))
    );
    assert_eq!(attached.get("mcp_servers"), Some(&json(r#"["srv-a"]"#)));
    assert_eq!(
        attached.get("mcp_tool_permissions"),
        Some(&string(r#"{"srv-a": ["t1"]}"#))
    );
    assert_eq!(attached.get("skills"), Some(&json("[]")));
    assert!(!attached.contains_key("old"));

    let mut absent =
        fields(r#"{"object_permission_id": "missing", "object_permission": {"old": true}}"#);
    attach_object_permission_to_dict(&mut absent, Some(database.prisma()))
        .await
        .unwrap();
    assert_eq!(
        absent.get("object_permission"),
        Some(&json(r#"{"old": true}"#))
    );
    database.close().await;
}

#[tokio::test]
async fn prepare_no_write() {
    let database = TestDatabase::start().await;
    database
        .seed_permission("perm-1", r#"["srv-a"]"#, None)
        .await;
    let incoming = fields(r#"{"models": ["m"]}"#);
    let prepared =
        prepare_object_permission_upsert(&incoming, Some("perm-1"), database.prisma(), &[])
            .await
            .unwrap();
    assert_eq!(prepared.object_permission_id, "perm-1");
    assert_eq!(
        prepared.record.get("mcp_servers"),
        Some(&json(r#"["srv-a"]"#))
    );
    assert_eq!(prepared.record.get("models"), Some(&json(r#"["m"]"#)));
    assert_eq!(database.permission_count().await, 1);
    assert_eq!(
        database
            .permission_row("perm-1")
            .await
            .unwrap()
            .get("models"),
        Some(&JsonData::Null)
    );

    let fresh = prepare_object_permission_upsert(&incoming, None, database.prisma(), &[])
        .await
        .unwrap();
    assert_ne!(fresh.object_permission_id, "perm-1");
    assert_generated_id(&fresh.object_permission_id);
    assert!(
        database
            .permission_row(&fresh.object_permission_id)
            .await
            .is_none()
    );
    assert_eq!(database.permission_count().await, 1);
    database.close().await;
}

#[tokio::test]
async fn pinned_upsert_id() {
    let database = TestDatabase::start().await;
    database
        .seed_permission("perm-1", r#"["srv-a"]"#, None)
        .await;
    let incoming = fields(r#"{"object_permission_id": "override", "models": ["m"]}"#);
    let kept = prepare_object_permission_upsert(&incoming, Some("perm-1"), database.prisma(), &[])
        .await
        .unwrap();
    assert_eq!(kept.object_permission_id, "perm-1");
    assert_eq!(
        kept.record.get("object_permission_id"),
        Some(&string("perm-1"))
    );

    let minted = prepare_object_permission_upsert(&incoming, Some(""), database.prisma(), &[])
        .await
        .unwrap();
    assert_ne!(minted.object_permission_id, "");
    assert_generated_id(&minted.object_permission_id);
    assert_eq!(
        minted.record.get("object_permission_id"),
        Some(&string(&minted.object_permission_id))
    );
    assert!(
        database
            .permission_row(&minted.object_permission_id)
            .await
            .is_none()
    );
    database.close().await;
}

#[tokio::test]
async fn shallow_replace_and_null() {
    let database = TestDatabase::start().await;
    database
        .seed_permission(
            "perm-1",
            r#"["a"]"#,
            Some(r#"{"srv-a": ["t1"], "srv-b": ["t2"]}"#),
        )
        .await;
    database
        .execute(
            r#"UPDATE "LiteLLM_ObjectPermissionTable" SET agents = ARRAY['agent-a'] WHERE object_permission_id = 'perm-1'"#,
            &[],
        )
        .await;
    let incoming = fields(
        r#"{"mcp_servers": ["b"], "models": null, "mcp_tool_permissions": {"srv-a": ["t1"]}}"#,
    );
    let prepared =
        prepare_object_permission_upsert(&incoming, Some("perm-1"), database.prisma(), &[])
            .await
            .unwrap();
    assert_eq!(prepared.record.get("mcp_servers"), Some(&json(r#"["b"]"#)));
    assert_eq!(prepared.record.get("models"), Some(&JsonData::Null));
    assert_eq!(prepared.record.get("agents"), Some(&json(r#"["agent-a"]"#)));
    assert_eq!(
        prepared.record.get("mcp_tool_permissions"),
        Some(&string(r#"{"srv-a":["t1"]}"#))
    );

    let mut data = BTreeMap::new();
    data.insert(
        "object_permission".to_owned(),
        JsonData::Object(incoming.clone()),
    );
    let returned = handle_update_object_permission_common(
        &mut data,
        Some("perm-1"),
        Some(database.prisma()),
        &[],
    )
    .await
    .unwrap();
    assert_eq!(returned.as_deref(), Some("perm-1"));
    let row = database.permission_row("perm-1").await.unwrap();
    assert_eq!(row.get("models"), Some(&JsonData::Null));
    assert_eq!(row.get("mcp_servers"), Some(&json(r#"["b"]"#)));
    assert_eq!(
        row.get("mcp_tool_permissions"),
        Some(&string(r#"{"srv-a":["t1"]}"#))
    );
    database.close().await;
}

#[tokio::test]
async fn safe_json_tool_map() {
    let database = TestDatabase::start().await;
    let incoming = fields(r#"{"mcp_tool_permissions": {"srv-b": ["t2", "t1"]}}"#);
    let prepared = prepare_object_permission_upsert(&incoming, None, database.prisma(), &[])
        .await
        .unwrap();
    assert_eq!(
        prepared.record.get("mcp_tool_permissions"),
        Some(&string(r#"{"srv-b":["t2","t1"]}"#))
    );

    let null_update = fields(r#"{"mcp_tool_permissions": null}"#);
    let prepared = prepare_object_permission_upsert(&null_update, None, database.prisma(), &[])
        .await
        .unwrap();
    assert_eq!(
        prepared.record.get("mcp_tool_permissions"),
        Some(&string("null"))
    );

    // A stored grant is already the column's JSON string and is not encoded a second time
    database
        .seed_permission("perm-1", r#"[]"#, Some(r#"{"srv-a": ["t1"]}"#))
        .await;
    let unrelated = fields(r#"{"mcp_servers": ["srv-b"]}"#);
    let prepared =
        prepare_object_permission_upsert(&unrelated, Some("perm-1"), database.prisma(), &[])
            .await
            .unwrap();
    assert_eq!(
        prepared.record.get("mcp_tool_permissions"),
        Some(&string(r#"{"srv-a": ["t1"]}"#))
    );

    let echoed = fields(r#"{"mcp_tool_permissions": "{\"srv-a\": [\"t2\"]}"}"#);
    let prepared =
        prepare_object_permission_upsert(&echoed, Some("perm-1"), database.prisma(), &[])
            .await
            .unwrap();
    assert_eq!(
        prepared.record.get("mcp_tool_permissions"),
        Some(&string(r#"{"srv-a": ["t2"]}"#))
    );
    database.close().await;
}

#[tokio::test]
async fn update_pop_before_parse() {
    let mut data = fields(r#"{"team_id": "t", "object_permission": {"mcp_servers": ["a"]}}"#);
    let error = handle_update_object_permission_common(&mut data, None, None, &[])
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), "Prisma client not found");
    assert!(data.contains_key("object_permission"));

    let database = TestDatabase::start().await;
    let mut invalid = fields(r#"{"object_permission": "{", "team_id": "t"}"#);
    let error =
        handle_update_object_permission_common(&mut invalid, None, Some(database.prisma()), &[])
            .await
            .unwrap_err();
    assert!(matches!(error, ObjectPermissionError::Document(_)));
    assert!(!invalid.contains_key("object_permission"));
    assert_eq!(invalid.get("team_id"), Some(&string("t")));
    assert_eq!(database.permission_count().await, 0);

    let mut explicit_null = fields(r#"{"object_permission": null}"#);
    let returned = handle_update_object_permission_common(
        &mut explicit_null,
        None,
        Some(database.prisma()),
        &[],
    )
    .await
    .unwrap();
    assert_eq!(returned, None);
    assert!(!explicit_null.contains_key("object_permission"));
    let mut missing = fields(r#"{"team_id": "t"}"#);
    let returned =
        handle_update_object_permission_common(&mut missing, None, Some(database.prisma()), &[])
            .await
            .unwrap();
    assert_eq!(returned, None);
    assert_eq!(missing, fields(r#"{"team_id": "t"}"#));
    assert_eq!(database.permission_count().await, 0);
    database.close().await;
}

#[rstest]
#[case::number("7")]
#[case::boolean("true")]
#[case::array("[1, 2]")]
#[case::numeric_string("\"5\"")]
#[case::null_string("\"null\"")]
#[tokio::test]
async fn non_object_empty_upsert(#[case] raw: &str) {
    let database = TestDatabase::start().await;
    let mut data = permission_input(raw);
    let returned =
        handle_update_object_permission_common(&mut data, None, Some(database.prisma()), &[])
            .await
            .unwrap();
    let Some(id) = returned else {
        panic!("a non-object permission still creates an empty row");
    };
    assert_generated_id(&id);
    let row = database.permission_row(&id).await.unwrap();
    assert_eq!(row.get("object_permission_id"), Some(&string(&id)));
    assert_eq!(row.get("mcp_servers"), Some(&json("[]")));
    assert_eq!(row.get("mcp_tool_permissions"), Some(&JsonData::Null));
    assert!(!data.contains_key("object_permission"));
    assert_eq!(data.get("object_permission_id"), Some(&string(&id)));
    assert_eq!(database.permission_count().await, 1);
    database.close().await;
}

#[tokio::test]
async fn upsert_before_parent_failure() {
    let database = TestDatabase::start().await;
    let mut data = fields(r#"{"team_id": "t", "object_permission": {"mcp_servers": ["srv-a"]}}"#);
    let returned =
        handle_update_object_permission_common(&mut data, None, Some(database.prisma()), &[])
            .await
            .unwrap();
    let Some(id) = returned else {
        panic!("the permission upsert returns its id");
    };
    let failure = database.insert_parent("bad", &id).await.unwrap_err();
    assert!(
        failure
            .to_string()
            .contains("B816PermissionParents_checked")
    );
    let row = database.permission_row(&id).await.unwrap();
    assert_eq!(row.get("mcp_servers"), Some(&json(r#"["srv-a"]"#)));
    assert_eq!(data.get("object_permission_id"), Some(&string(&id)));
    database.close().await;
}

#[tokio::test]
async fn create_null_cleaning() {
    let database = TestDatabase::start().await;
    let mut data = fields(
        r#"{"team_id": "t", "object_permission": {"object_permission_id": "caller", "models": null, "mcp_servers": ["srv-a"], "mcp_tool_permissions": null}}"#,
    );
    set_object_permission(&mut data, Some(database.prisma()), &[])
        .await
        .unwrap();
    let Some(JsonData::String(id)) = data.get("object_permission_id") else {
        panic!("create links the generated id");
    };
    let id = id.clone();
    assert_ne!(id, "caller");
    assert_generated_id(&id);
    let row = database.permission_row(&id).await.unwrap();
    assert_eq!(row.get("object_permission_id"), Some(&string(&id)));
    assert_eq!(row.get("mcp_servers"), Some(&json(r#"["srv-a"]"#)));
    assert_eq!(row.get("models"), Some(&JsonData::Null));
    assert_eq!(row.get("mcp_tool_permissions"), Some(&JsonData::Null));
    assert!(!data.contains_key("object_permission"));
    assert_eq!(database.permission_row("caller").await, None);

    let mut untouched = permission_input(r#"{"mcp_servers": ["a"]}"#);
    set_object_permission(&mut untouched, None, &[])
        .await
        .unwrap();
    assert_eq!(untouched, permission_input(r#"{"mcp_servers": ["a"]}"#));
    let mut absent = fields(r#"{"team_id": "t"}"#);
    set_object_permission(&mut absent, Some(database.prisma()), &[])
        .await
        .unwrap();
    assert_eq!(absent, fields(r#"{"team_id": "t"}"#));
    assert_eq!(database.permission_count().await, 1);
    database.close().await;
}

#[tokio::test]
async fn create_empty_record() {
    let database = TestDatabase::start().await;
    let mut empty = permission_input("{}");
    set_object_permission(&mut empty, Some(database.prisma()), &[])
        .await
        .unwrap();
    let Some(JsonData::String(id)) = empty.get("object_permission_id") else {
        panic!("an empty permission still creates a row");
    };
    assert_generated_id(id);
    let row = database.permission_row(id).await.unwrap();
    assert_eq!(row.get("mcp_servers"), Some(&json("[]")));
    assert_eq!(row.get("skills"), Some(&json("[]")));

    // Without json.loads a string is not a dict, so the field is dropped instead of becoming a record
    let mut text = fields(r#"{"object_permission": "{}", "team_id": "t"}"#);
    set_object_permission(&mut text, Some(database.prisma()), &[])
        .await
        .unwrap();
    assert!(!text.contains_key("object_permission"));
    assert_eq!(text.get("team_id"), Some(&string("t")));
    let mut listed = fields(r#"{"object_permission": [1, 2], "team_id": "t"}"#);
    set_object_permission(&mut listed, Some(database.prisma()), &[])
        .await
        .unwrap();
    assert!(!listed.contains_key("object_permission"));
    assert!(!listed.contains_key("object_permission_id"));
    assert_eq!(database.permission_count().await, 1);
    database.close().await;
}

#[tokio::test]
async fn create_failure_keeps_input() {
    let database = TestDatabase::start().await;
    database.drop_permission_table().await;
    let mut data = fields(r#"{"team_id": "t", "object_permission": {"mcp_servers": ["srv-a"]}}"#);
    let error = set_object_permission(&mut data, Some(database.prisma()), &[])
        .await
        .unwrap_err();
    assert!(matches!(error, ObjectPermissionError::Database(_)));
    assert_eq!(
        data,
        fields(r#"{"team_id": "t", "object_permission": {"mcp_servers": ["srv-a"]}}"#)
    );
    assert!(!data.contains_key("object_permission_id"));
    database.close().await;
}

#[tokio::test]
async fn changed_collision_400() {
    let database = TestDatabase::start().await;
    database.seed_server("srv-1", Some("shared"), None).await;
    database.seed_server("srv-2", Some("shared"), None).await;
    let mut data = permission_input(r#"{"mcp_tool_permissions": {"shared": ["t1"]}}"#);
    let error = set_object_permission(&mut data, Some(database.prisma()), &[])
        .await
        .unwrap_err();
    assert_eq!(error.status_code(), Some(StatusCode::BAD_REQUEST));
    assert_eq!(
        error.to_string(),
        collision_message("'shared' matches MCP servers ['srv-1', 'srv-2']")
    );
    assert_eq!(
        data,
        permission_input(r#"{"mcp_tool_permissions": {"shared": ["t1"]}}"#)
    );
    assert_eq!(database.permission_count().await, 0);

    // A changed empty tool set on a stored legacy collision is rejected like a new entry
    database
        .seed_permission("perm-1", r#"[]"#, Some(r#"{"shared": ["t1"]}"#))
        .await;
    let mut update = permission_input(r#"{"mcp_tool_permissions": {"shared": []}}"#);
    let error = handle_update_object_permission_common(
        &mut update,
        Some("perm-1"),
        Some(database.prisma()),
        &[],
    )
    .await
    .unwrap_err();
    assert_eq!(error.status_code(), Some(StatusCode::BAD_REQUEST));
    assert_eq!(
        database
            .permission_row("perm-1")
            .await
            .unwrap()
            .get("mcp_tool_permissions"),
        Some(&string(r#"{"shared": ["t1"]}"#))
    );
    assert_eq!(database.permission_count().await, 1);
    database.close().await;
}

#[tokio::test]
async fn grandfathered_collision() {
    let database = TestDatabase::start().await;
    database.seed_server("srv-1", Some("shared"), None).await;
    database.seed_server("srv-2", Some("shared"), None).await;
    database
        .seed_permission("perm-1", r#"["srv-a"]"#, Some(r#"{"shared": ["t1"]}"#))
        .await;

    let mut unrelated = permission_input(r#"{"mcp_servers": ["srv-b"]}"#);
    let returned = handle_update_object_permission_common(
        &mut unrelated,
        Some("perm-1"),
        Some(database.prisma()),
        &[],
    )
    .await
    .unwrap();
    assert_eq!(returned.as_deref(), Some("perm-1"));
    let row = database.permission_row("perm-1").await.unwrap();
    assert_eq!(row.get("mcp_servers"), Some(&json(r#"["srv-b"]"#)));
    assert_eq!(
        row.get("mcp_tool_permissions"),
        Some(&string(r#"{"shared": ["t1"]}"#))
    );

    let mut echoed = permission_input(r#"{"mcp_tool_permissions": {"shared": ["t1"]}}"#);
    handle_update_object_permission_common(
        &mut echoed,
        Some("perm-1"),
        Some(database.prisma()),
        &[],
    )
    .await
    .unwrap();
    let mut deleted = permission_input(r#"{"mcp_tool_permissions": {}}"#);
    handle_update_object_permission_common(
        &mut deleted,
        Some("perm-1"),
        Some(database.prisma()),
        &[],
    )
    .await
    .unwrap();
    assert_eq!(
        database
            .permission_row("perm-1")
            .await
            .unwrap()
            .get("mcp_tool_permissions"),
        Some(&string("{}"))
    );
    assert_eq!(database.permission_count().await, 1);
    database.close().await;
}

#[tokio::test]
async fn exact_id_wins() {
    let database = TestDatabase::start().await;
    database.seed_server("srv-1", Some("shared"), None).await;
    let by_alias = registry_server(
        r#"{"server_id": "srv-2", "name": "display-2", "alias": "srv-1", "transport": "http"}"#,
    );
    let same_name = registry_server(
        r#"{"server_id": "srv-3", "name": "display-3", "server_name": "shared", "transport": "http"}"#,
    );
    let registry = [&by_alias, &same_name];

    let mut exact = permission_input(r#"{"mcp_tool_permissions": {"srv-1": ["t1"]}}"#);
    set_object_permission(&mut exact, Some(database.prisma()), &registry)
        .await
        .unwrap();
    assert!(exact.contains_key("object_permission_id"));

    let mut collision = permission_input(r#"{"mcp_tool_permissions": {"shared": ["t1"]}}"#);
    let error = set_object_permission(&mut collision, Some(database.prisma()), &registry)
        .await
        .unwrap_err();
    assert_eq!(error.status_code(), Some(StatusCode::BAD_REQUEST));
    assert_eq!(
        error.to_string(),
        collision_message("'shared' matches MCP servers ['srv-1', 'srv-3']")
    );
    assert_eq!(database.permission_count().await, 1);
    database.close().await;
}

#[tokio::test]
async fn unresolved_identifier() {
    let database = TestDatabase::start().await;
    let mut data = permission_input(r#"{"mcp_tool_permissions": {"ghost": ["t1"]}}"#);
    let returned =
        handle_update_object_permission_common(&mut data, None, Some(database.prisma()), &[])
            .await
            .unwrap();
    let Some(id) = returned else {
        panic!("an unresolved identifier is still stored");
    };
    let row = database.permission_row(&id).await.unwrap();
    assert_eq!(
        row.get("mcp_tool_permissions"),
        Some(&string(r#"{"ghost":["t1"]}"#))
    );
    database.close().await;
}

#[tokio::test]
async fn cache_id_filter_order() {
    let eviction = RecordingEviction::default();
    let endpoint = Arc::new(RecordingEndpoint::default());
    let tasks = PublishTasks::default();
    invalidate_cached_object_permissions(
        &[
            Some("first"),
            Some(""),
            None,
            Some("first"),
            Some("second"),
            Some(""),
        ],
        &eviction,
        &tasks,
        Some(&endpoint),
    )
    .await;
    let messages = wait_for_publishes(&endpoint, 3).await;
    tasks.shutdown().await;
    assert_eq!(
        *eviction.keys.lock().unwrap(),
        [
            "object_permission_id:first",
            "object_permission_id:",
            "object_permission_id:second"
        ]
    );
    assert_eq!(
        messages,
        [
            r#"{"cache_key":"object_permission_id:first"}"#,
            r#"{"cache_key":"object_permission_id:"}"#,
            r#"{"cache_key":"object_permission_id:second"}"#
        ]
    );

    let empty_eviction = RecordingEviction::default();
    let quiet = PublishTasks::default();
    invalidate_cached_object_permissions(&[], &empty_eviction, &quiet, None).await;
    quiet.shutdown().await;
    assert!(empty_eviction.keys.lock().unwrap().is_empty());
}

#[tokio::test]
async fn eviction_failure() {
    let failing = RecordingEviction::failing();
    let endpoint = Arc::new(RecordingEndpoint::default());
    let tasks = PublishTasks::default();
    invalidate_cached_object_permissions(
        &[Some("first"), Some("second")],
        &failing,
        &tasks,
        Some(&endpoint),
    )
    .await;
    let messages = wait_for_publishes(&endpoint, 2).await;
    tasks.shutdown().await;
    assert_eq!(
        *failing.keys.lock().unwrap(),
        ["object_permission_id:first", "object_permission_id:second"]
    );
    assert_eq!(
        messages,
        [
            r#"{"cache_key":"object_permission_id:first"}"#,
            r#"{"cache_key":"object_permission_id:second"}"#
        ]
    );

    let quiet = PublishTasks::default();
    invalidate_cached_object_permissions(
        &[Some("third")],
        &RecordingEviction::failing(),
        &quiet,
        None,
    )
    .await;
    quiet.shutdown().await;
}

#[tokio::test]
async fn postgres_pinned_rows() {
    let database = TestDatabase::start().await;
    let mut created = permission_input(
        r#"{"object_permission_id": "override", "mcp_servers": ["srv-a"], "mcp_tool_permissions": {"srv-a": ["t1"]}}"#,
    );
    let returned =
        handle_update_object_permission_common(&mut created, None, Some(database.prisma()), &[])
            .await
            .unwrap();
    let Some(id) = returned else {
        panic!("the upsert returns the pinned id");
    };
    assert_ne!(id, "override");
    assert_eq!(created.get("object_permission_id"), Some(&string(&id)));
    let row = database.permission_row(&id).await.unwrap();
    assert_eq!(row.get("object_permission_id"), Some(&string(&id)));
    assert_eq!(row.get("mcp_servers"), Some(&json(r#"["srv-a"]"#)));
    assert_eq!(
        row.get("mcp_tool_permissions"),
        Some(&string(r#"{"srv-a":["t1"]}"#))
    );

    let mut updated = permission_input(r#"{"models": ["m"], "object_permission_id": "other"}"#);
    let returned = handle_update_object_permission_common(
        &mut updated,
        Some(&id),
        Some(database.prisma()),
        &[],
    )
    .await
    .unwrap();
    assert_eq!(returned.as_deref(), Some(id.as_str()));
    let row = database.permission_row(&id).await.unwrap();
    assert_eq!(row.get("models"), Some(&json(r#"["m"]"#)));
    assert_eq!(row.get("mcp_servers"), Some(&json(r#"["srv-a"]"#)));
    assert_eq!(
        row.get("mcp_tool_permissions"),
        Some(&string(r#"{"srv-a":["t1"]}"#))
    );
    assert_eq!(database.permission_count().await, 1);
    database.close().await;
}

#[tokio::test]
async fn postgres_failure_effects() {
    let database = TestDatabase::start().await;
    database.seed_server("srv-1", Some("shared"), None).await;
    database.seed_server("srv-2", Some("shared"), None).await;
    let mut ambiguous = permission_input(r#"{"mcp_tool_permissions": {"shared": ["t1"]}}"#);
    let error =
        handle_update_object_permission_common(&mut ambiguous, None, Some(database.prisma()), &[])
            .await
            .unwrap_err();
    assert_eq!(error.status_code(), Some(StatusCode::BAD_REQUEST));
    assert!(!ambiguous.contains_key("object_permission"));
    assert_eq!(database.permission_count().await, 0);

    database.drop_permission_table().await;
    let mut failed_update =
        fields(r#"{"team_id": "t", "object_permission": {"mcp_servers": ["a"]}}"#);
    let error = handle_update_object_permission_common(
        &mut failed_update,
        None,
        Some(database.prisma()),
        &[],
    )
    .await
    .unwrap_err();
    assert!(matches!(error, ObjectPermissionError::Database(_)));
    assert_eq!(failed_update, fields(r#"{"team_id": "t"}"#));

    let original = fields(r#"{"team_id": "t", "object_permission": {"mcp_servers": ["a"]}}"#);
    let mut failed_create = original.clone();
    let error = set_object_permission(&mut failed_create, Some(database.prisma()), &[])
        .await
        .unwrap_err();
    assert!(matches!(error, ObjectPermissionError::Database(_)));
    assert_eq!(failed_create, original);
    database.close().await;
}

#[test]
fn permission_json_byte_and_depth_limits() {
    let at_bytes = format!("null{}", " ".repeat(0x000F_FFFC));
    assert_eq!(at_bytes.len(), 0x0010_0000);
    assert_eq!(
        decode_object_permission(at_bytes.as_bytes()).unwrap().json,
        JsonData::Null
    );
    let over_bytes = format!("null{}", " ".repeat(0x000F_FFFD));
    assert_eq!(over_bytes.len(), 0x0010_0001);
    assert!(matches!(
        decode_object_permission(over_bytes.as_bytes()),
        Err(DecodeError::Bounded(PrivateJsonError::TooLarge(
            0x0010_0001,
            0x0010_0000
        )))
    ));

    let at_depth = format!("{}{}", "[".repeat(64), "]".repeat(64));
    assert_eq!(
        decode_object_permission(at_depth.as_bytes()).unwrap().json,
        json(&at_depth)
    );
    let over_depth = format!("{}{}", "[".repeat(65), "]".repeat(65));
    assert!(matches!(
        decode_object_permission(over_depth.as_bytes()),
        Err(DecodeError::Bounded(PrivateJsonError::TooDeep(64)))
    ));

    assert!(matches!(
        decode_object_permission(br#"{"a": [1"#),
        Err(DecodeError::Bounded(PrivateJsonError::Decode(_)))
    ));
    // B192's loader parses through serde_json::Value, which keeps the last value of a duplicate key
    assert_eq!(
        decode_object_permission(br#"{"a": 1, "a": 2}"#)
            .unwrap()
            .json,
        json(r#"{"a": 2}"#)
    );
    assert_eq!(decode_object_permission(b" 7 ").unwrap().json, json("7"));
}

#[test]
fn entries_project_raw_forms() {
    assert!(mcp_tool_permission_entries(None).unwrap().is_empty());
    assert!(
        mcp_tool_permission_entries(Some(&json("null")))
            .unwrap()
            .is_empty()
    );
    assert!(
        mcp_tool_permission_entries(Some(&json("true")))
            .unwrap()
            .is_empty()
    );
    assert!(
        mcp_tool_permission_entries(Some(&json("7")))
            .unwrap()
            .is_empty()
    );
    assert!(
        mcp_tool_permission_entries(Some(&json("[1, 2]")))
            .unwrap()
            .is_empty()
    );
    let parsed = mcp_tool_permission_entries(Some(&json(
        r#"{"srv-a": ["t2", "t1", "t1"], "srv-b": null, "srv-c": []}"#,
    )))
    .unwrap();
    let names: Vec<&str> = parsed["srv-a"].iter().map(String::as_str).collect();
    assert_eq!(names, ["t1", "t2"]);
    assert!(parsed["srv-b"].is_empty());
    assert!(parsed["srv-c"].is_empty());
    let from_text = mcp_tool_permission_entries(Some(&string(r#"{"srv-a": ["t1"]}"#))).unwrap();
    let from_text_names: Vec<&str> = from_text["srv-a"].iter().map(String::as_str).collect();
    assert_eq!(from_text_names, ["t1"]);
    assert!(mcp_tool_permission_entries(Some(&string("{"))).is_err());
    assert!(mcp_tool_permission_entries(Some(&string("null"))).is_err());
    assert!(mcp_tool_permission_entries(Some(&json(r#"{"srv-a": "t1"}"#))).is_err());
    assert!(mcp_tool_permission_entries(Some(&json(r#"{"srv-a": [1]}"#))).is_err());
}

#[test]
fn stored_row_fields_drop_nulls() {
    let row = json(
        r#"{"object_permission_id": "perm-1", "models": null, "mcp_servers": ["a"], "mcp_tool_permissions": null}"#,
    );
    let kept = non_null_fields(&row);
    assert!(!kept.contains_key("models"));
    assert!(!kept.contains_key("mcp_tool_permissions"));
    assert_eq!(kept.get("mcp_servers"), Some(&json(r#"["a"]"#)));
    assert_eq!(kept.get("object_permission_id"), Some(&string("perm-1")));
    assert!(non_null_fields(&json("[]")).is_empty());
}

#[rstest]
#[case::empty_object("{}")]
#[case::null_id(r#"{"object_permission_id": null}"#)]
#[case::number_id(r#"{"object_permission_id": 7}"#)]
#[case::array("[]")]
#[case::null("null")]
fn stored_row_id_projection(#[case] raw: &str) {
    let error = object_permission_id_of(&json(raw)).unwrap_err();
    assert!(matches!(
        error,
        ObjectPermissionError::MissingObjectPermissionId
    ));
    assert_eq!(error.status_code(), None);
}

#[test]
fn stored_row_id_kept() {
    let id = object_permission_id_of(&json(r#"{"object_permission_id": "perm-1"}"#)).unwrap();
    assert_eq!(id, "perm-1");
}

#[rstest]
fn replays_the_object_permissions_corpus(
    #[files("fuzz/corpus/object_permissions/*")] path: PathBuf,
) {
    let bytes = fs::read(path).unwrap();
    drop(decode_object_permission(&bytes));
}
