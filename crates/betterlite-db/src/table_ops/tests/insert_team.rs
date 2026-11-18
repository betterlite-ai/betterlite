#![cfg(test)]
use super::support::{Harness, Ports, TableClient, json, team_payload, text_column};
use crate::budget_permission_rows::JsonData;
use crate::key_repository::table::TokenValue;
use crate::table_ops::columns::TeamColumn;
use crate::table_ops::inserts::TableInserts as _;

#[tokio::test]
async fn insert_team_creates_the_row_and_keeps_an_existing_one() {
    let harness = Harness::start().await;
    let db = harness.pool.get().await.expect("a fixture connection");
    let client = TableClient::new(harness.pool.clone());
    let ports = Ports::new();
    let ops = ports.operations(&client);
    let created = ops
        .insert_team(&team_payload("team-1", "acme"))
        .await
        .expect("the team insert runs");
    assert_eq!(created.fields.base.team_alias.as_deref(), Some("acme"));
    let kept = ops
        .insert_team(&team_payload("team-1", "other"))
        .await
        .expect("the conflicting insert runs");
    assert_eq!(kept.fields.base.team_alias.as_deref(), Some("acme"));
    let stored = text_column(
        &db,
        "SELECT team_alias FROM \"LiteLLM_TeamTable\" WHERE team_id = $1",
        "team-1",
    )
    .await;
    assert_eq!(stored.as_deref(), Some("acme"));
    let count: i64 = db
        .query_one("SELECT count(*) FROM \"LiteLLM_TeamTable\"", &[])
        .await
        .expect("the count runs")
        .get(0);
    assert_eq!(count, 1);
    assert!(ports.sink.events().is_empty());
    harness.close().await;
}
#[tokio::test]
async fn insert_team_dumps_the_map_fields_to_json_text() {
    let harness = Harness::start().await;
    let db = harness.pool.get().await.expect("a fixture connection");
    let client = TableClient::new(harness.pool.clone());
    let ports = Ports::new();
    let ops = ports.operations(&client);
    let mut data = team_payload("team-2", "maps");
    data.insert(
        TeamColumn::Metadata,
        TokenValue::Json(json(r#"{"department":"eng"}"#)),
    );
    data.insert(TeamColumn::MembersWithRoles, TokenValue::Json(json("[]")));
    data.insert(TeamColumn::BudgetLimits, TokenValue::Json(json("[]")));
    let created = ops.insert_team(&data).await.expect("the team insert runs");
    let metadata = created
        .fields
        .base
        .metadata
        .expect("the stored metadata decodes");
    assert_eq!(
        metadata.get("department"),
        Some(&JsonData::String(String::from("eng")))
    );
    assert!(created.fields.base.members_with_roles.is_empty());
    assert_eq!(created.fields.base.budget_limits, Some(Vec::new()));
    let row = db
        .query_one(
            "SELECT jsonb_typeof(metadata), jsonb_typeof(members_with_roles), jsonb_typeof(budget_limits) FROM \"LiteLLM_TeamTable\" WHERE team_id = 'team-2'",
            &[],
        )
        .await
        .expect("the type query runs");
    assert_eq!(row.get::<_, Option<String>>(0).as_deref(), Some("string"));
    assert_eq!(row.get::<_, Option<String>>(1).as_deref(), Some("string"));
    assert_eq!(row.get::<_, Option<String>>(2).as_deref(), Some("string"));
    harness.close().await;
}
