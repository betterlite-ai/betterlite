#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_core::agents::permissions::AgentObjectPermission;
use betterlite_db::budget_permission_rows::JsonData;

use super::PermissionValue;
use super::converted_record;
use super::prepare;
use super::write_permission;
use crate::agent_persistence::support::Database;
use crate::agent_persistence::support::permission_column_json;

fn permission(json: &str) -> AgentObjectPermission {
    serde_json::from_str(json).expect("a valid object permission")
}

#[test]
fn converted_record_carries_the_tool_search_flag() {
    let record = BTreeMap::from([(
        String::from("mcp_tool_search_enabled"),
        JsonData::Bool(true),
    )]);
    let converted = converted_record(&record).expect("the record converts");
    assert!(
        converted
            .iter()
            .any(|(name, value)| *name == "mcp_tool_search_enabled"
                && matches!(value, PermissionValue::Flag(Some(true))))
    );
}

#[tokio::test]
async fn permission_writes_the_present_columns() {
    let database = Database::start().await;
    let store = database.store();
    let requested = permission(r#"{"mcp_servers": ["srv"], "models": ["gpt-4o"]}"#);
    let prepared = prepare(&store, Some(&requested), None)
        .await
        .expect("the permission row is prepared")
        .expect("the permission is present");
    let mut client = store.session().await.expect("a session");
    let tx = client.transaction().await.expect("a transaction");
    write_permission(&tx, &prepared)
        .await
        .expect("the permission row is written");
    tx.commit().await.expect("the transaction commits");
    let servers = database
        .json(
            r#"SELECT to_jsonb(mcp_servers)::text FROM "LiteLLM_ObjectPermissionTable" WHERE object_permission_id = $1"#,
            &[&prepared.object_permission_id],
        )
        .await;
    assert_eq!(
        servers,
        Some(JsonData::Array(vec![JsonData::String(String::from("srv"))]))
    );
    let models = database
        .json(
            r#"SELECT to_jsonb(models)::text FROM "LiteLLM_ObjectPermissionTable" WHERE object_permission_id = $1"#,
            &[&prepared.object_permission_id],
        )
        .await;
    assert_eq!(
        models,
        Some(JsonData::Array(vec![JsonData::String(String::from(
            "gpt-4o"
        ))]))
    );
    database.close().await;
}

#[tokio::test]
async fn permission_stores_tool_permissions_as_a_json_string() {
    let database = Database::start().await;
    let store = database.store();
    let requested = permission(r#"{"mcp_tool_permissions": {"srv": ["t1"]}}"#);
    let prepared = prepare(&store, Some(&requested), None)
        .await
        .expect("the permission row is prepared")
        .expect("the permission is present");
    let mut client = store.session().await.expect("a session");
    let tx = client.transaction().await.expect("a transaction");
    write_permission(&tx, &prepared)
        .await
        .expect("the permission row is written");
    tx.commit().await.expect("the transaction commits");
    let tools = database
        .json(
            r#"SELECT mcp_tool_permissions::text FROM "LiteLLM_ObjectPermissionTable" WHERE object_permission_id = $1"#,
            &[&prepared.object_permission_id],
        )
        .await;
    assert_eq!(
        tools,
        Some(JsonData::String(String::from(r#"{"srv":["t1"]}"#)))
    );
    database.close().await;
}

#[tokio::test]
async fn permission_keeps_a_stored_column_the_update_leaves_out() {
    let database = Database::start().await;
    let store = database.store();
    let first = permission(r#"{"mcp_servers": ["old"], "mcp_access_groups": ["group"]}"#);
    let prepared = prepare(&store, Some(&first), None)
        .await
        .expect("the permission row is prepared")
        .expect("the permission is present");
    let mut client = store.session().await.expect("a session");
    let tx = client.transaction().await.expect("a transaction");
    write_permission(&tx, &prepared)
        .await
        .expect("the permission row is written");
    tx.commit().await.expect("the transaction commits");
    let second = permission(r#"{"mcp_servers": ["new"]}"#);
    let updated = prepare(&store, Some(&second), Some(&prepared.object_permission_id))
        .await
        .expect("the same row is prepared again")
        .expect("the permission is present");
    assert_eq!(updated.object_permission_id, prepared.object_permission_id);
    let mut updating = store.session().await.expect("a session");
    let updating_tx = updating.transaction().await.expect("a transaction");
    write_permission(&updating_tx, &updated)
        .await
        .expect("the permission row is updated");
    updating_tx.commit().await.expect("the transaction commits");
    let permission_id = prepared.object_permission_id.as_str();
    let servers = permission_column_json(&database, "mcp_servers", permission_id).await;
    assert_eq!(
        servers,
        Some(JsonData::Array(vec![JsonData::String(String::from("new"))]))
    );
    let groups = permission_column_json(&database, "mcp_access_groups", permission_id).await;
    assert_eq!(
        groups,
        Some(JsonData::Array(vec![JsonData::String(String::from(
            "group"
        ))]))
    );
    database.close().await;
}

#[tokio::test]
async fn permission_write_rolls_back_with_the_transaction() {
    let database = Database::start().await;
    let store = database.store();
    let requested = permission(r#"{"mcp_servers": ["srv"]}"#);
    let prepared = prepare(&store, Some(&requested), None)
        .await
        .expect("the permission row is prepared")
        .expect("the permission is present");
    let mut client = store.session().await.expect("a session");
    let tx = client.transaction().await.expect("a transaction");
    write_permission(&tx, &prepared)
        .await
        .expect("the permission row is written");
    tx.rollback().await.expect("the transaction rolls back");
    assert_eq!(
        database
            .count(
                r#"SELECT count(*)::bigint FROM "LiteLLM_ObjectPermissionTable" WHERE object_permission_id = $1"#,
                &[&prepared.object_permission_id],
            )
            .await,
        0
    );
    database.close().await;
}

#[tokio::test]
async fn permission_writes_an_empty_record_twice() {
    let database = Database::start().await;
    let store = database.store();
    let empty = permission("{}");
    let prepared = prepare(&store, Some(&empty), None)
        .await
        .expect("the empty permission is prepared")
        .expect("the permission is present");
    let mut client = store.session().await.expect("a session");
    let tx = client.transaction().await.expect("a transaction");
    write_permission(&tx, &prepared)
        .await
        .expect("the empty row is written");
    write_permission(&tx, &prepared)
        .await
        .expect("the empty row is written again");
    tx.commit().await.expect("the transaction commits");
    assert_eq!(
        database
            .count(
                r#"SELECT count(*)::bigint FROM "LiteLLM_ObjectPermissionTable" WHERE object_permission_id = $1"#,
                &[&prepared.object_permission_id],
            )
            .await,
        1
    );
    database.close().await;
}
