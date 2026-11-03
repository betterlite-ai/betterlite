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
    let (sql, params) = build_bulk_upsert(
        &DailySpendEntity::Tag.table(),
        &[(key, row)],
        uuid::Uuid::nil,
    )
    .unwrap();
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
    first.batch_execute(DAILY_SCHEMA).await.unwrap();
    let mut initial = row("tag", Null, 2);
    initial.insert("model_group".to_owned(), Text("first".to_owned()));
    initial.insert("request_id".to_owned(), Text("request".to_owned()));
    write_bulk_upsert(
        &first,
        DailySpendEntity::Tag,
        &[initial],
        uuid::Uuid::new_v4,
    )
    .await
    .unwrap();
    let rows = [row("tag", Null, 3), row("tag", Text(String::new()), 4)];
    let (left, right) = tokio::join!(
        write_bulk_upsert(&first, DailySpendEntity::Tag, &rows, uuid::Uuid::new_v4),
        write_bulk_upsert(&second, DailySpendEntity::Tag, &rows, uuid::Uuid::new_v4)
    );
    left.unwrap();
    right.unwrap();
    check_daily(&first).await;
    replace_request_and_skip_empty(&first).await;
    drop((first, second));
    pool.close();
    fixture.close_async().await.unwrap();
}

const DAILY_SCHEMA: &str = r#"CREATE TABLE "LiteLLM_DailyTagSpend" (
    id text PRIMARY KEY, tag text, date text NOT NULL, api_key text NOT NULL,
    model text, model_group text, custom_llm_provider text, mcp_namespaced_tool_name text,
    endpoint text, request_id text, updated_at timestamp(3) NOT NULL,
    created_at timestamp(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    prompt_tokens bigint NOT NULL DEFAULT 0, completion_tokens bigint NOT NULL DEFAULT 0,
    api_requests bigint NOT NULL DEFAULT 0, successful_requests bigint NOT NULL DEFAULT 0,
    failed_requests bigint NOT NULL DEFAULT 0, cache_read_input_tokens bigint NOT NULL DEFAULT 0,
    cache_creation_input_tokens bigint NOT NULL DEFAULT 0, compression_saved_tokens bigint NOT NULL DEFAULT 0,
    total_response_time_ms bigint NOT NULL DEFAULT 0, timed_requests bigint NOT NULL DEFAULT 0,
    spend double precision NOT NULL DEFAULT 0, compression_savings_spend double precision NOT NULL DEFAULT 0,
    prompt_caching_savings_spend double precision NOT NULL DEFAULT 0,
    gateway_injected_caching_savings_spend double precision NOT NULL DEFAULT 0,
    autorouter_savings_spend double precision NOT NULL DEFAULT 0,
    UNIQUE(tag, date, api_key, model, custom_llm_provider, mcp_namespaced_tool_name, endpoint)
)"#;

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
    let result = first.query_one("SELECT prompt_tokens, spend::numeric, model_group, request_id, model, count(*) OVER (), id FROM \"LiteLLM_DailyTagSpend\"", &[]).await.unwrap();
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
    let identifier = uuid::Uuid::parse_str(&result.get::<_, String>(6)).unwrap();
    assert_eq!(identifier.get_version(), Some(uuid::Version::Random));
}

#[rstest]
#[case::oversized(vec![b' '; 0x0010_0001])]
#[case::invalid_utf8(vec![0xff])]
fn rejects_invalid_tag_bytes(#[case] bytes: Vec<u8>) {
    decode_daily_tags(&bytes).unwrap_err();
}

#[test]
fn sums_money_without_binary_roundoff() {
    let mut first = row("tag", Null, 0);
    first.insert(
        "spend".to_owned(),
        Decimal(rust_decimal::Decimal::new(1, 1)),
    );
    let mut second = row("tag", Null, 0);
    second.insert(
        "spend".to_owned(),
        Decimal(rust_decimal::Decimal::new(2, 1)),
    );
    let groups = merge_by_conflict_key(&DailySpendEntity::Tag.table(), &[first, second]).unwrap();
    assert_eq!(
        groups[0].1.get("spend"),
        Some(&Decimal(rust_decimal::Decimal::new(3, 1)))
    );
}

#[rstest]
#[case::fraction("0.1234567890123456789", "0.1234567890123456789")]
#[case::zero("0", "0.0")]
#[case::whole("100", "100.0")]
#[case::maximum("79228162514264337593543950335", "79228162514264337593543950335")]
fn exact_decimal_survives_column_and_bind_serialization(
    #[case] token: &str,
    #[case] expected: &str,
) {
    let amount = token.parse::<rust_decimal::Decimal>().unwrap();
    let column = Decimal(amount);
    let encoded = serde_json::to_string(&column).unwrap();
    assert_eq!(encoded, expected);
    assert_eq!(
        serde_json::from_str::<super::values::SpendColumnValue>(&encoded).unwrap(),
        column
    );
    let bind = serde_json::to_string(&super::bind::SqlBind::Money(amount)).unwrap();
    assert_eq!(bind, expected);
    assert_eq!(
        serde_json::from_str::<super::values::SpendColumnValue>(&bind).unwrap(),
        column
    );
}

#[test]
fn reserved_number_object_is_nonnumeric() {
    let source = r#"{"spend":{"$serde_json::private::Number":"100"},"prompt_tokens":{"$serde_json::private::Number":"2"}}"#;
    let object: SpendRow = serde_json::from_str(source).unwrap();
    let numeric: SpendRow = serde_json::from_str(r#"{"spend":3,"prompt_tokens":4}"#).unwrap();
    assert!(matches!(
        object.get("spend"),
        Some(super::values::SpendColumnValue::Object(_))
    ));
    let groups =
        merge_by_conflict_key(&DailySpendEntity::Team.table(), &[object, numeric]).unwrap();
    assert_eq!(
        groups[0].1.get("spend"),
        Some(&Decimal(rust_decimal::Decimal::from(3_i32)))
    );
    assert_eq!(groups[0].1.get("prompt_tokens"), Some(&Int(4)));
}

#[test]
fn uses_injected_ids_in_batch_order() {
    let groups = merge_by_conflict_key(
        &DailySpendEntity::Team.table(),
        &serde_json::from_str::<Vec<SpendRow>>(r#"[{"team_id":"b"},{"team_id":"a"}]"#).unwrap(),
    )
    .unwrap();
    let mut identifiers = [uuid::Uuid::from_u128(1), uuid::Uuid::from_u128(2)].into_iter();
    let (_, params) = build_bulk_upsert(&DailySpendEntity::Team.table(), &groups, || {
        identifiers.next().unwrap()
    })
    .unwrap();
    assert_eq!(
        serde_json::to_string(&params[0]).unwrap(),
        r#""00000000-0000-0000-0000-000000000001""#
    );
    assert_eq!(serde_json::to_string(&params[1]).unwrap(), r#""a""#);
    assert_eq!(
        serde_json::to_string(&params[24]).unwrap(),
        r#""00000000-0000-0000-0000-000000000002""#
    );
    assert_eq!(serde_json::to_string(&params[25]).unwrap(), r#""b""#);
    assert_eq!(identifiers.next(), None);
    let (_, empty) = build_bulk_upsert(&DailySpendEntity::Team.table(), &[], || {
        identifiers.next().unwrap()
    })
    .unwrap();
    assert!(empty.is_empty());
}

async fn replace_request_and_skip_empty(client: &deadpool_postgres::Client) {
    use super::write::write_bulk_upsert;
    let mut replacement = row("tag", Null, 0);
    replacement.insert("request_id".to_owned(), Text("new".to_owned()));
    write_bulk_upsert(
        client,
        DailySpendEntity::Tag,
        &[replacement],
        uuid::Uuid::new_v4,
    )
    .await
    .unwrap();
    let updated = client
        .query_one("SELECT request_id FROM \"LiteLLM_DailyTagSpend\"", &[])
        .await
        .unwrap();
    assert_eq!(updated.get::<_, String>(0), "new");
    client
        .batch_execute("DROP TABLE \"LiteLLM_DailyTagSpend\"")
        .await
        .unwrap();
    write_bulk_upsert(client, DailySpendEntity::Tag, &[], uuid::Uuid::new_v4)
        .await
        .unwrap();
}

#[rstest]
#[case::arrays("[", "]", 65)]
#[case::objects(r#"{"nested":"#, "}", 65)]
#[case::mixed(r#"[{"nested":"#, "}]", 33)]
#[case::deep_arrays("[", "]", 4096)]
#[case::deep_objects(r#"{"nested":"#, "}", 4096)]
#[case::deep_mixed(r#"[{"nested":"#, "}]", 2048)]
fn rejects_excessive_daily_nesting(
    #[case] prefix: &str,
    #[case] suffix: &str,
    #[case] count: usize,
) {
    let text = format!("{}null{}", prefix.repeat(count), suffix.repeat(count));
    let raw = serde_json::value::RawValue::from_string(text.clone()).unwrap();
    assert!(matches!(
        super::values::SpendColumnValue::try_from(raw),
        Err(super::values::DailySpendValueError::TooDeep)
    ));
    let row_json = format!(r#"{{"spend":{text}}}"#);
    assert!(serde_json::from_str::<SpendRow>(&row_json).is_err());
}

#[rstest]
#[case::arrays("[", "]", 64)]
#[case::objects(r#"{"nested":"#, "}", 64)]
#[case::mixed(r#"[{"nested":"#, "}]", 32)]
fn accepts_daily_nesting_boundary(
    #[case] prefix: &str,
    #[case] suffix: &str,
    #[case] count: usize,
) {
    let source = format!("{}null{}", prefix.repeat(count), suffix.repeat(count));
    let decoded = super::value_decode::decode_daily_value(source.as_bytes()).unwrap();
    assert_eq!(serde_json::to_string(&decoded).unwrap(), source);
    let through_serde: super::values::SpendColumnValue = serde_json::from_str(&source).unwrap();
    assert_eq!(through_serde, decoded);
}

#[test]
fn daily_siblings_keep_their_depth_budget() {
    let child = format!("{}0{}", "[".repeat(63), "]".repeat(63));
    let source = format!("[{child},{child}]");
    let decoded = super::value_decode::decode_daily_value(source.as_bytes()).unwrap();
    assert_eq!(serde_json::to_string(&decoded).unwrap(), source);
}

#[test]
fn daily_value_size_is_bounded() {
    use super::value_decode::{MAX_DAILY_VALUE_BYTES, decode_daily_value};
    use super::values::{DailySpendValueError, SpendColumnValue};
    let source = format!("\"{}\"", "x".repeat(MAX_DAILY_VALUE_BYTES - 2));
    let decoded = decode_daily_value(source.as_bytes()).unwrap();
    assert_eq!(serde_json::to_string(&decoded).unwrap(), source);
    let oversized = format!("\"{}\"", "x".repeat(MAX_DAILY_VALUE_BYTES - 1));
    assert!(matches!(
        decode_daily_value(oversized.as_bytes()),
        Err(DailySpendValueError::TooLarge)
    ));
    let raw = serde_json::value::RawValue::from_string(oversized).unwrap();
    assert!(matches!(
        SpendColumnValue::try_from(raw),
        Err(DailySpendValueError::TooLarge)
    ));
}

#[rstest]
fn replays_the_daily_values_corpus(#[files("fuzz/corpus/daily_values/*")] path: PathBuf) {
    drop(super::value_decode::decode_daily_value(
        &read(path).unwrap(),
    ));
}
