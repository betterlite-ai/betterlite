use betterlite_db::budget_permission_rows::LiteLLMBudgetTable;
use rstest::rstest;

use super::support::{
    FakeCache, Logs, auth, dec, decoded_row, json, member_fixture, membership_row, only,
};

#[tokio::test]
async fn member_cache_reads_the_reservation_key() {
    let fixture = member_fixture();
    fixture.cache.put(
        "team_membership:user-a:team-a",
        &membership_row(r#"{"max_budget":10}"#, "1"),
    );
    let list = fixture.counters().await;
    let counter = only(&list);
    assert_eq!(counter.counter_key, "spend:team_member:user-a:team-a");
    assert_eq!(counter.entity_id, "user-a:team-a");
    assert_eq!(counter.entity_type.wire_value(), "team_member");
    assert_eq!(counter.max_budget, dec("10"));
    assert_eq!(counter.fallback_spend, dec("1"));
    assert_eq!(
        counter.source_cache_key.as_deref(),
        Some("team_membership:user-a:team-a")
    );
    assert_eq!(
        fixture.cache.read_keys(),
        vec!["team_membership:user-a:team-a".to_owned()]
    );
}

#[tokio::test]
async fn member_cache_failure_falls_open_and_warns() {
    let (logs, _guard) = Logs::capture();
    let mut fixture = member_fixture();
    fixture.cache = FakeCache::failing();
    assert!(fixture.counters().await.is_empty());
    assert_eq!(
        fixture.cache.read_keys(),
        vec!["team_membership:user-a:team-a".to_owned()]
    );
    let text = logs.text();
    assert!(text.contains("Skipping a reservation counter"), "{text}");
    assert!(text.contains("entity=team membership"), "{text}");
}

#[rstest]
#[case::active(
    r#"{"max_budget":10,"temp_budget_increase":2,"temp_budget_expiry":"2024-01-03T00:00:00Z"}"#,
    "12"
)]
#[case::expired(
    r#"{"max_budget":10,"temp_budget_increase":2,"temp_budget_expiry":"2023-12-31T00:00:00Z"}"#,
    "10"
)]
#[case::no_expiry(r#"{"max_budget":10,"temp_budget_increase":2}"#, "10")]
#[tokio::test]
async fn member_effective_adds_only_an_active_temp_increase(
    #[case] budget: &str,
    #[case] expected: &str,
) {
    let fixture = member_fixture();
    fixture.cache.put(
        "team_membership:user-a:team-a",
        &membership_row(budget, "0"),
    );
    let list = fixture.counters().await;
    let counter = only(&list);
    assert_eq!(counter.max_budget, dec(expected));
}

#[tokio::test]
async fn member_temp_increase_applies_to_the_default_budget() {
    let mut fixture = member_fixture();
    fixture.team = Some(decoded_row(
        r#"{"team_id":"team-a","metadata":{"team_member_budget_id":"default-1"}}"#,
    ));
    fixture.cache.put(
        "team_membership:user-a:team-a",
        &membership_row(
            concat!(
                r#"{"max_budget":null,"temp_budget_increase":2,"#,
                r#""temp_budget_expiry":"2024-01-03T00:00:00Z"}"#,
            ),
            "0",
        ),
    );
    let default_budget: LiteLLMBudgetTable = decoded_row(r#"{"max_budget":5}"#);
    fixture
        .cache
        .put("team_member_default_budget:default-1", &default_budget);
    let list = fixture.counters().await;
    let counter = only(&list);
    assert_eq!(counter.max_budget, dec("7"));
    assert_eq!(
        fixture.cache.read_keys(),
        vec![
            "team_membership:user-a:team-a".to_owned(),
            "team_member_default_budget:default-1".to_owned(),
        ]
    );
}

#[tokio::test]
async fn member_temp_increase_ignores_the_default_rows_own_increase() {
    let mut fixture = member_fixture();
    fixture.team = Some(decoded_row(
        r#"{"team_id":"team-a","metadata":{"team_member_budget_id":"default-1"}}"#,
    ));
    let default_budget: LiteLLMBudgetTable = decoded_row(concat!(
        r#"{"max_budget":5,"temp_budget_increase":3,"#,
        r#""temp_budget_expiry":"2024-01-03T00:00:00Z"}"#,
    ));
    fixture
        .cache
        .put("team_member_default_budget:default-1", &default_budget);
    let list = fixture.counters().await;
    let counter = only(&list);
    assert_eq!(counter.max_budget, dec("5"));
}

#[tokio::test]
async fn member_token_user_skip_without_a_token_user_id() {
    let mut fixture = member_fixture();
    fixture.token = auth(r#"{"token":"tok"}"#);
    fixture.cache.put(
        "team_membership:user-a:team-a",
        &membership_row(r#"{"max_budget":10}"#, "0"),
    );
    assert!(fixture.counters().await.is_empty());
    assert!(fixture.cache.read_keys().is_empty());
}

#[tokio::test]
async fn member_skip_without_a_team() {
    let mut fixture = member_fixture();
    fixture.team = None;
    fixture.cache.put(
        "team_membership:user-a:team-a",
        &membership_row(r#"{"max_budget":10}"#, "0"),
    );
    assert!(fixture.counters().await.is_empty());
    assert!(fixture.cache.read_keys().is_empty());
}

#[tokio::test]
async fn member_skip_without_a_user_row() {
    let mut fixture = member_fixture();
    fixture.user = None;
    fixture.cache.put(
        "team_membership:user-a:team-a",
        &membership_row(r#"{"max_budget":10}"#, "0"),
    );
    assert!(fixture.counters().await.is_empty());
    assert!(fixture.cache.read_keys().is_empty());
}

#[tokio::test]
async fn member_full_budget_table_supplies_the_cap() {
    let fixture = member_fixture();
    fixture.cache.put(
        "team_membership:user-a:team-a",
        &membership_row(
            r#"{"max_budget":10,"budget_id":"budget-a","created_at":"2024-01-01T00:00:00Z"}"#,
            "2",
        ),
    );
    let list = fixture.counters().await;
    let counter = only(&list);
    assert_eq!(counter.max_budget, dec("10"));
    assert_eq!(counter.fallback_spend, dec("2"));
}

#[tokio::test]
async fn member_skip_non_string_default_budget_id() {
    let mut fixture = member_fixture();
    fixture.team = Some(decoded_row(
        r#"{"team_id":"team-a","metadata":{"team_member_budget_id":7}}"#,
    ));
    fixture.cache.put(
        "team_membership:user-a:team-a",
        &membership_row(r#"{"max_budget":null}"#, "0"),
    );
    let stored: LiteLLMBudgetTable = decoded_row(r#"{"max_budget":5}"#);
    fixture.cache.put("team_member_default_budget:7", &stored);
    assert!(fixture.counters().await.is_empty());
    assert_eq!(
        fixture.cache.read_keys(),
        vec!["team_membership:user-a:team-a".to_owned()]
    );
}

#[tokio::test]
async fn member_effective_overflow_skips_the_counter() {
    let fixture = member_fixture();
    fixture.cache.put_encoded(
        "team_membership:user-a:team-a",
        json(concat!(
            r#"{"user_id":"user-a","team_id":"team-a","spend":0,"#,
            r#""litellm_budget_table":{"max_budget":"79228162514264337593543950335","#,
            r#""temp_budget_increase":1,"temp_budget_expiry":"2024-01-03T00:00:00Z"}}"#,
        )),
    );
    assert!(fixture.counters().await.is_empty());
}

#[rstest]
#[case::no_rows(None, None)]
#[case::zero_membership_budget(Some(r#"{"max_budget":0}"#), None)]
#[case::zero_default_budget(Some(r#"{"max_budget":null}"#), Some(r#"{"max_budget":0}"#))]
#[case::negative_default_budget(Some(r#"{"max_budget":null}"#), Some(r#"{"max_budget":-5}"#))]
#[case::zero_default_with_an_active_increase(
    Some(
        r#"{"max_budget":null,"temp_budget_increase":2,"temp_budget_expiry":"2024-01-03T00:00:00Z"}"#
    ),
    Some(r#"{"max_budget":0}"#)
)]
#[case::increase_cancels_the_cap(
    Some(
        r#"{"max_budget":1,"temp_budget_increase":-2,"temp_budget_expiry":"2024-01-03T00:00:00Z"}"#
    ),
    None
)]
#[tokio::test]
async fn member_skip_zero_or_missing_budget(
    #[case] membership: Option<&str>,
    #[case] default_row: Option<&str>,
) {
    let mut fixture = member_fixture();
    fixture.team = Some(decoded_row(
        r#"{"team_id":"team-a","metadata":{"team_member_budget_id":"default-1"}}"#,
    ));
    if let Some(budget) = membership {
        fixture.cache.put(
            "team_membership:user-a:team-a",
            &membership_row(budget, "0"),
        );
    }
    if let Some(spec) = default_row {
        let stored: LiteLLMBudgetTable = decoded_row(spec);
        fixture
            .cache
            .put("team_member_default_budget:default-1", &stored);
    }
    assert!(fixture.counters().await.is_empty());
}
