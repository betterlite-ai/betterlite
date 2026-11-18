#![cfg(test)]
use super::support::{Harness, Ports, TableClient};
use crate::key_repository::table::TokenValue;
use crate::table_ops::columns::TeamColumn;
use crate::table_ops::inserts::TableInserts as _;
use std::collections::BTreeMap;
use tokio_postgres::Client;

fn team_payload(team_id: &str, alias: &str) -> BTreeMap<TeamColumn, TokenValue> {
    BTreeMap::from([
        (TeamColumn::TeamId, TokenValue::Text(team_id.to_owned())),
        (TeamColumn::TeamAlias, TokenValue::Text(alias.to_owned())),
        (TeamColumn::Admins, TokenValue::TextArray(Vec::new())),
        (TeamColumn::Members, TokenValue::TextArray(Vec::new())),
        (TeamColumn::Models, TokenValue::TextArray(Vec::new())),
    ])
}

async fn text_column(db: &Client, sql: &str, parameter: &str) -> Option<String> {
    db.query_opt(sql, &[&parameter])
        .await
        .expect("the column query runs")
        .map(|row| row.get(0))
}

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
