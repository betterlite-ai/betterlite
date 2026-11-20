use rstest::rstest;

use super::support::{Fixture, auth, dec, decoded_row, only};

#[tokio::test]
async fn enduser_token_id_resolves_from_the_token() {
    let mut fixture = Fixture::new();
    fixture.token = auth(r#"{"token":"tok","end_user_id":"eu-1","end_user_max_budget":50}"#);
    let list = fixture.counters().await;
    let counter = only(&list);
    assert_eq!(counter.counter_key, "spend:end_user:eu-1");
    assert_eq!(counter.max_budget, dec("50"));
    assert_eq!(counter.fallback_spend, dec("0"));
    assert_eq!(counter.entity_type.wire_value(), "end_user");
    assert_eq!(
        counter.source_cache_key.as_deref(),
        Some("end_user_id:eu-1")
    );
}

#[tokio::test]
async fn enduser_param_id_wins_over_the_token() {
    let mut fixture = Fixture::new();
    fixture.token = auth(r#"{"token":"tok","end_user_id":"eu-token","end_user_max_budget":50}"#);
    fixture.end_user_id = Some("eu-param".to_owned());
    let list = fixture.counters().await;
    let counter = only(&list);
    assert_eq!(counter.counter_key, "spend:end_user:eu-param");
    assert_eq!(counter.entity_id, "eu-param");
    assert_eq!(
        counter.source_cache_key.as_deref(),
        Some("end_user_id:eu-param")
    );
}

#[tokio::test]
async fn enduser_fallback_table_supplies_the_cap_and_spend() {
    let mut fixture = Fixture::new();
    fixture.token = auth(r#"{"token":"tok","end_user_id":"eu-1"}"#);
    fixture.end_user = Some(decoded_row(
        r#"{"user_id":"eu-1","blocked":false,"spend":4,"litellm_budget_table":{"max_budget":40}}"#,
    ));
    let list = fixture.counters().await;
    let counter = only(&list);
    assert_eq!(counter.max_budget, dec("40"));
    assert_eq!(counter.fallback_spend, dec("4"));
}

#[rstest]
#[case::no_id(r#"{"token":"tok"}"#, None)]
#[case::no_budget_from_anywhere(r#"{"token":"tok","end_user_id":"eu-1"}"#, None)]
#[case::zero_token_budget_keeps_no_fallback(
    r#"{"token":"tok","end_user_id":"eu-1","end_user_max_budget":0}"#,
    Some(
        r#"{"user_id":"eu-1","blocked":false,"spend":4,"litellm_budget_table":{"max_budget":40}}"#
    )
)]
#[case::zero_table_budget(
    r#"{"token":"tok","end_user_id":"eu-1"}"#,
    Some(
        r#"{"user_id":"eu-1","blocked":false,"spend":4,"litellm_budget_table":{"max_budget":0}}"#
    )
)]
#[case::negative_token_budget(
    r#"{"token":"tok","end_user_id":"eu-1","end_user_max_budget":-5}"#,
    None
)]
#[tokio::test]
async fn enduser_skip_zero_or_missing_budget(#[case] token: &str, #[case] end_user: Option<&str>) {
    let mut fixture = Fixture::new();
    fixture.token = auth(token);
    fixture.end_user = end_user.map(decoded_row);
    assert!(fixture.counters().await.is_empty());
}
