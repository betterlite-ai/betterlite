#![cfg(test)]

use super::builder::build_bulk_upsert;
use super::entities::{
    DailySpendEntity, DailySpendEntityPayload, SpendRow, daily_spend_entity_ids,
};
use super::merge::merge_by_conflict_key;
use super::tags::decode_daily_tags;
use super::values::SpendColumnValue::{Decimal, Int, Null, Text};
use crate::autorouter_turn_sql::tests::database;
use rstest::rstest;
use std::fs::read;
use std::path::PathBuf;

fn row(tag: &str, model: super::values::SpendColumnValue, requests: i64) -> SpendRow {
    let mut row = SpendRow::new();
    row.insert("tag".to_owned(), Text(tag.to_owned()));
    row.insert("date".to_owned(), Text("2026-01-01".to_owned()));
    row.insert("api_key".to_owned(), Null);
    row.insert("model".to_owned(), model);
    row.insert("prompt_tokens".to_owned(), Int(requests));
    row.insert(
        "spend".to_owned(),
        Decimal(rust_decimal::Decimal::new(15, 1)),
    );
    row
}

#[test]
fn null_empty_keys_merge_and_keep_first_metadata() {
    let mut first = row("tag", Null, 2);
    first.insert("model_group".to_owned(), Text("first".to_owned()));
    let mut second = row("tag", Text(String::new()), 3);
    second.insert("model_group".to_owned(), Text("second".to_owned()));
    let batch = merge_by_conflict_key(&DailySpendEntity::Tag.table(), &[first, second]).unwrap();
    assert_eq!(batch.len(), 1);
    assert_eq!(
        batch[0].1.get("model_group"),
        Some(&Text("first".to_owned()))
    );
    assert_eq!(batch[0].1.get("prompt_tokens"), Some(&Int(5)));
}

#[test]
fn tag_upsert_keeps_request_id_and_uses_fixed_table() {
    let mut row = row("tag", Text("m".to_owned()), 1);
    row.insert("request_id".to_owned(), Null);
    let key = super::merge::conflict_key(&DailySpendEntity::Tag.table(), &row).unwrap();
    let (sql, params) = build_bulk_upsert(&DailySpendEntity::Tag.table(), &[(key, row)]).unwrap();
    assert!(
        sql.contains("COALESCE(EXCLUDED.\"request_id\", \"LiteLLM_DailyTagSpend\".\"request_id\")")
    );
    assert!(!sql.contains("model_group\" = EXCLUDED"));
    assert_eq!(params.len(), 25);
}

#[test]
fn user_null_and_end_user_empty_are_distinct() {
    let payload = DailySpendEntityPayload {
        user: Some(None),
        team_id: None,
        organization_id: None,
        end_user: Some(String::new()),
        agent_id: None,
        request_tags: None,
    };
    assert_eq!(
        daily_spend_entity_ids(&payload, DailySpendEntity::User).unwrap(),
        vec![None]
    );
    assert!(
        daily_spend_entity_ids(&payload, DailySpendEntity::EndUser)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn tag_json_deduplicates_and_rejects_non_strings() {
    assert_eq!(
        decode_daily_tags(br#"["a","b","a"]"#).unwrap(),
        vec!["a", "b"]
    );
    decode_daily_tags(br#"["a",2]"#).unwrap_err();
}

#[rstest]
fn replays_the_daily_tags_corpus(#[files("fuzz/corpus/daily_tags/*")] path: PathBuf) {
    drop(decode_daily_tags(&read(path).unwrap()));
}

#[tokio::test]
async fn postgres_daily_collisions_and_tag_request_retention() {
    use super::write::write_bulk_upsert;
    let (fixture, pool) = database().await;
    let first = pool.get().await.unwrap();
    let second = pool.get().await.unwrap();
    daily_schema(&first).await;
    let mut initial = row("tag", Null, 2);
    initial.insert("model_group".to_owned(), Text("first".to_owned()));
    initial.insert("request_id".to_owned(), Text("request".to_owned()));
    write_bulk_upsert(&first, DailySpendEntity::Tag, &[initial])
        .await
        .unwrap();
    let rows = [row("tag", Null, 3), row("tag", Text(String::new()), 4)];
    let (left, right) = tokio::join!(
        write_bulk_upsert(&first, DailySpendEntity::Tag, &rows),
        write_bulk_upsert(&second, DailySpendEntity::Tag, &rows)
    );
    left.unwrap();
    right.unwrap();
    check_daily(&first).await;
    let mut replacement = row("tag", Null, 0);
    replacement.insert("request_id".to_owned(), Text("new".to_owned()));
    write_bulk_upsert(&first, DailySpendEntity::Tag, &[replacement])
        .await
        .unwrap();
    let updated = first
        .query_one("SELECT request_id FROM \"LiteLLM_DailyTagSpend\"", &[])
        .await
        .unwrap();
    assert_eq!(updated.get::<_, String>(0), "new");
    first
        .batch_execute("DROP TABLE \"LiteLLM_DailyTagSpend\"")
        .await
        .unwrap();
    write_bulk_upsert(&first, DailySpendEntity::Tag, &[])
        .await
        .unwrap();
    drop((first, second));
    pool.close();
    fixture.close_async().await.unwrap();
}

async fn daily_schema(client: &deadpool_postgres::Client) {
    let mut fields = vec![
        "id text PRIMARY KEY",
        "tag text",
        "date text",
        "api_key text",
        "model text",
        "custom_llm_provider text",
        "mcp_namespaced_tool_name text",
        "endpoint text",
        "model_group text",
        "request_id text",
        "updated_at timestamp",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect::<Vec<_>>();
    fields.extend(
        super::columns::COUNTER_COLUMNS
            .iter()
            .map(|column| format!("{column} bigint NOT NULL")),
    );
    fields.extend(
        super::columns::SPEND_COLUMNS
            .iter()
            .map(|column| format!("{column} double precision NOT NULL")),
    );
    fields.push("UNIQUE(tag, date, api_key, model, custom_llm_provider, mcp_namespaced_tool_name, endpoint)".to_owned());
    client
        .batch_execute(&format!(
            "CREATE TABLE \"LiteLLM_DailyTagSpend\" ({})",
            fields.join(", ")
        ))
        .await
        .unwrap();
}

#[test]
fn rejects_overflow_and_nonfinite_money() {
    let rows = [row("tag", Null, i64::MAX), row("tag", Null, 1)];
    merge_by_conflict_key(&DailySpendEntity::Tag.table(), &rows).unwrap_err();
    let huge = Decimal(rust_decimal::Decimal::MAX);
    super::values::as_counter(Some(&huge), "prompt_tokens").unwrap_err();
    serde_json::from_str::<SpendRow>(r#"{"spend":1e999}"#).unwrap_err();
}

#[test]
fn rolling_upgrade_daily_record_defaults() {
    let row: super::records::DailyTagSpendTransaction = serde_json::from_str(r#"{"tag":"t","request_id":null,"date":"2026-01-01","api_key":"hash","model":null,"model_group":null,"mcp_namespaced_tool_name":null,"custom_llm_provider":null,"endpoint":null,"prompt_tokens":1,"completion_tokens":2,"cache_read_input_tokens":0,"cache_creation_input_tokens":0,"compression_saved_tokens":0,"compression_savings_spend":0,"prompt_caching_savings_spend":0,"gateway_injected_caching_savings_spend":0,"spend":0.25,"api_requests":1,"successful_requests":1,"failed_requests":0}"#).unwrap();
    assert_eq!(
        row.base.autorouter_savings_spend,
        rust_decimal::Decimal::ZERO
    );
    assert_eq!(
        (row.base.total_response_time_ms, row.base.timed_requests),
        (0, 0)
    );
    assert_eq!(row.base.spend, rust_decimal::Decimal::new(25, 2));
    let encoded: SpendRow = serde_json::from_str(&serde_json::to_string(&row).unwrap()).unwrap();
    assert_eq!(
        encoded.get("spend"),
        Some(&Decimal(rust_decimal::Decimal::new(25, 2)))
    );
}

async fn check_daily(first: &deadpool_postgres::Client) {
    let result = first.query_one("SELECT prompt_tokens, spend::numeric, model_group, request_id, model, count(*) OVER () FROM \"LiteLLM_DailyTagSpend\"", &[]).await.unwrap();
    assert_eq!(
        (
            result.get::<_, i64>(0),
            result.get::<_, rust_decimal::Decimal>(1)
        ),
        (16, rust_decimal::Decimal::new(75, 1))
    );
    assert_eq!(
        (
            result.get::<_, String>(2),
            result.get::<_, String>(3),
            result.get::<_, String>(4),
            result.get::<_, i64>(5)
        ),
        ("first".to_owned(), "request".to_owned(), String::new(), 1)
    );
}
