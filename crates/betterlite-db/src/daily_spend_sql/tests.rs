#![cfg(test)]

use super::builder::build_bulk_upsert;
use super::entities::{
    DailySpendEntity, DailySpendEntityPayload, SpendRow, daily_spend_entity_ids,
};
use super::merge::merge_by_conflict_key;
use super::tags::decode_daily_tags;
use super::values::SpendColumnValue::{Float, Int, Null, Text};
use rstest::rstest;
use std::fs::read;
use std::path::Path;

fn row(tag: &str, model: super::values::SpendColumnValue, requests: i64) -> SpendRow {
    let mut row = SpendRow::new();
    row.insert("tag", Text(tag.to_owned()));
    row.insert("date", Text("2026-01-01".to_owned()));
    row.insert("api_key", Null);
    row.insert("model", model);
    row.insert("prompt_tokens", Int(requests));
    row.insert("spend", Float(1.5));
    row
}

#[test]
fn null_empty_keys_merge_and_keep_first_metadata() {
    let mut first = row("tag", Null, 2);
    first.insert("model_group", Text("first".to_owned()));
    let mut second = row("tag", Text(String::new()), 3);
    second.insert("model_group", Text("second".to_owned()));
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
    row.insert("request_id", Null);
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
#[case::array("array.json")]
#[case::invalid_member("non-string-member.json")]
#[case::text_array("text-wrapped-array.json")]
fn replays_the_daily_tags_corpus(#[case] filename: &str) {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/daily_tags");
    let bytes = read(corpus.join(filename)).unwrap();
    drop(decode_daily_tags(&bytes));
}
