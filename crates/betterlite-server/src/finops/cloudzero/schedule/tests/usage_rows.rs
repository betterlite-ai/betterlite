#![cfg(test)]

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::finops::cloudzero::UsageRow;
use betterlite_logging::finops::cloudzero::records::UsageRecord;
use rust_decimal::Decimal;

use super::fixtures::{decimal, integer, row, text};
use crate::finops::cloudzero::schedule::usage_rows::usage_record;

fn full_row() -> UsageRow {
    row(&[
        ("date", text("2026-01-01")),
        ("custom_llm_provider", text("openai")),
        ("team_id", text("team-1")),
        ("team_alias", text("team-alias")),
        ("model", text("gpt-4o")),
        ("model_group", text("group-1")),
        ("api_key", text("sk-key")),
        ("api_key_alias", text("key-alias")),
        ("user_email", text("user@example.com")),
        ("organization_alias", text("org")),
        ("project_alias", text("project")),
        ("user_alias", text("user")),
        ("api_requests", integer(4)),
        ("successful_requests", integer(3)),
        ("failed_requests", integer(1)),
        ("prompt_tokens", integer(10)),
        ("completion_tokens", integer(5)),
        ("cache_creation_input_tokens", integer(2)),
        ("cache_read_input_tokens", integer(7)),
        ("spend", decimal("1.25")),
    ])
}

#[test]
fn cloudzero_schedule() {
    let expected = UsageRecord {
        date: Some("2026-01-01".to_owned()),
        custom_llm_provider: "openai".to_owned(),
        team_id: "team-1".to_owned(),
        team_alias: Some("team-alias".to_owned()),
        model: "gpt-4o".to_owned(),
        model_group: "group-1".to_owned(),
        api_key: "sk-key".to_owned(),
        api_key_alias: Some("key-alias".to_owned()),
        user_email: Some("user@example.com".to_owned()),
        organization_alias: Some("org".to_owned()),
        project_alias: Some("project".to_owned()),
        user_alias: Some("user".to_owned()),
        api_requests: 4,
        successful_requests: 3,
        failed_requests: 1,
        prompt_tokens: 10,
        completion_tokens: 5,
        cache_creation_input_tokens: 2,
        cache_read_input_tokens: 7,
        spend: Decimal::new(125, 2),
    };
    let nulls = row(&[("date", JsonData::Null), ("spend", JsonData::Null)]);
    assert_eq!(usage_record(&full_row()).expect("a full row"), expected);
    assert_eq!(
        usage_record(&UsageRow::new()).expect("an empty row"),
        UsageRecord::default()
    );
    assert_eq!(
        usage_record(&nulls).expect("a row with nulls"),
        UsageRecord::default()
    );
}
