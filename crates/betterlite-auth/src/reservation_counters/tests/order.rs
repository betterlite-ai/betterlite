use rust_decimal::Decimal;

use super::support::{Fixture, auth, dec, full_fixture, only};

#[tokio::test]
async fn counter_order_places_windows_before_the_next_entity() {
    let list = full_fixture().counters().await;
    let ids: Vec<&str> = list
        .iter()
        .map(|counter| counter.entity_id.as_str())
        .collect();
    assert_eq!(
        ids,
        vec![
            "tok",
            "tok:1d",
            "team-a",
            "user-a",
            "eu-1",
            "tag-a",
            "group-a",
            "user-a:team-a",
            "org-a",
            "project-a",
        ]
    );
}

#[tokio::test]
async fn counter_keys_follow_the_litellm_formats() {
    let list = full_fixture().counters().await;
    let keys: Vec<&str> = list
        .iter()
        .map(|counter| counter.counter_key.as_str())
        .collect();
    assert_eq!(
        keys,
        vec![
            "spend:key:tok",
            "spend:key:tok:window:1d",
            "spend:team:team-a",
            "spend:user:user-a",
            "spend:end_user:eu-1",
            "spend:tag:tag-a",
            "spend:model_access_group:group-a",
            "spend:team_member:user-a:team-a",
            "spend:org:org-a",
            "spend:project:project-a",
        ]
    );
}

#[tokio::test]
async fn cache_keys_match_the_cache_constructors() {
    let list = full_fixture().counters().await;
    let sources: Vec<Option<&str>> = list
        .iter()
        .map(|counter| counter.source_cache_key.as_deref())
        .collect();
    assert_eq!(
        sources,
        vec![
            Some("tok"),
            None,
            Some("team_id:team-a"),
            Some("user-a"),
            Some("end_user_id:eu-1"),
            Some("tag:tag-a"),
            Some("model_access_group:group-a"),
            Some("team_membership:user-a:team-a"),
            Some("org_id:org-a:with_budget"),
            Some("project_id:project-a"),
        ]
    );
}

#[tokio::test]
async fn entity_types_match_the_litellm_names() {
    let list = full_fixture().counters().await;
    let types: Vec<&str> = list
        .iter()
        .map(|counter| counter.entity_type.wire_value())
        .collect();
    assert_eq!(
        types,
        vec![
            "key",
            "key",
            "team",
            "user",
            "end_user",
            "tag",
            "model_access_group",
            "team_member",
            "organization",
            "project",
        ]
    );
}

#[tokio::test]
async fn fallback_spend_reads_each_entity_row() {
    let list = full_fixture().counters().await;
    let spends: Vec<Decimal> = list.iter().map(|counter| counter.fallback_spend).collect();
    assert_eq!(
        spends,
        vec![
            dec("1"),
            dec("0"),
            dec("2"),
            dec("3"),
            dec("4"),
            dec("9"),
            dec("8"),
            dec("5"),
            dec("6"),
            dec("7"),
        ]
    );
}

#[tokio::test]
async fn fallback_spend_defaults_to_zero() {
    let mut fixture = Fixture::new();
    fixture.token = auth(r#"{"token":"tok","max_budget":5}"#);
    let list = fixture.counters().await;
    let counter = only(&list);
    assert_eq!(counter.fallback_spend, dec("0"));
}
