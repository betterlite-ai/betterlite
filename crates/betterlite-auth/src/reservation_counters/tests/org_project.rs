use rstest::rstest;

use super::support::{Fixture, auth, dec, decoded_row, json, only, org_row, project_row};

#[tokio::test]
async fn org_token_id_prefers_the_token_org_id() {
    let mut fixture = Fixture::new();
    fixture.token = auth(r#"{"token":"tok","org_id":"org-a"}"#);
    fixture.team = Some(decoded_row(
        r#"{"team_id":"team-a","organization_id":"org-team"}"#,
    ));
    fixture.cache.put(
        "org_id:org-a:with_budget",
        &org_row(r#"{"max_budget":15}"#, "6"),
    );
    fixture.cache.put(
        "org_id:org-team:with_budget",
        &org_row(r#"{"max_budget":99}"#, "0"),
    );
    let list = fixture.counters().await;
    let counter = only(&list);
    assert_eq!(counter.counter_key, "spend:org:org-a");
    assert_eq!(counter.entity_id, "org-a");
    assert_eq!(counter.entity_type.wire_value(), "organization");
    assert_eq!(counter.max_budget, dec("15"));
}

#[tokio::test]
async fn org_team_id_falls_back_to_the_team_organization() {
    let mut fixture = Fixture::new();
    fixture.token = auth(r#"{"token":"tok"}"#);
    fixture.team = Some(decoded_row(
        r#"{"team_id":"team-a","organization_id":"org-team"}"#,
    ));
    fixture.cache.put(
        "org_id:org-team:with_budget",
        &org_row(r#"{"max_budget":15}"#, "6"),
    );
    let list = fixture.counters().await;
    let counter = only(&list);
    assert_eq!(counter.counter_key, "spend:org:org-team");
    assert_eq!(counter.entity_id, "org-team");
    assert_eq!(counter.max_budget, dec("15"));
}

#[tokio::test]
async fn org_cache_reads_the_with_budget_key() {
    let mut fixture = Fixture::new();
    fixture.token = auth(r#"{"token":"tok","org_id":"org-a"}"#);
    fixture.cache.put(
        "org_id:org-a:with_budget",
        &org_row(r#"{"max_budget":15}"#, "6"),
    );
    let list = fixture.counters().await;
    let counter = only(&list);
    assert_eq!(counter.fallback_spend, dec("6"));
    assert_eq!(
        counter.source_cache_key.as_deref(),
        Some("org_id:org-a:with_budget")
    );
    assert_eq!(
        fixture.cache.read_keys(),
        vec!["org_id:org-a:with_budget".to_owned()]
    );
    assert!(counter.spend_log_entity_id.is_none());
}

#[rstest]
#[case::missing_row(None)]
#[case::no_budget_table(Some("null"))]
#[case::zero_budget(Some(r#"{"max_budget":0}"#))]
#[case::negative_budget(Some(r#"{"max_budget":-4}"#))]
#[tokio::test]
async fn org_skip_zero_or_missing_budget(#[case] budget: Option<&str>) {
    let mut fixture = Fixture::new();
    fixture.token = auth(r#"{"token":"tok","org_id":"org-a"}"#);
    if let Some(spec) = budget {
        fixture
            .cache
            .put("org_id:org-a:with_budget", &org_row(spec, "0"));
    }
    assert!(fixture.counters().await.is_empty());
}

#[tokio::test]
async fn project_id_loads_the_cached_row() {
    let mut fixture = Fixture::new();
    fixture.token = auth(r#"{"token":"tok","project_id":"project-a"}"#);
    fixture.cache.put(
        "project_id:project-a",
        &project_row(r#"{"max_budget":12}"#, "7"),
    );
    let list = fixture.counters().await;
    let counter = only(&list);
    assert_eq!(counter.counter_key, "spend:project:project-a");
    assert_eq!(counter.entity_id, "project-a");
    assert_eq!(counter.entity_type.wire_value(), "project");
    assert_eq!(counter.max_budget, dec("12"));
    assert_eq!(counter.fallback_spend, dec("7"));
    assert_eq!(
        counter.source_cache_key.as_deref(),
        Some("project_id:project-a")
    );
    assert_eq!(
        fixture.cache.read_keys(),
        vec!["project_id:project-a".to_owned()]
    );
}

#[tokio::test]
async fn project_finite_rejects_unrepresentable_budgets() {
    let mut fixture = Fixture::new();
    fixture.token = auth(r#"{"token":"tok","project_id":"project-a"}"#);
    fixture.cache.put_encoded(
        "project_id:project-a",
        json(concat!(
            r#"{"project_id":"project-a","spend":0,"#,
            r#""litellm_budget_table":{"max_budget":1e40},"last_refreshed_at":null}"#,
        )),
    );
    assert!(fixture.counters().await.is_empty());
    assert_eq!(
        fixture.cache.read_keys(),
        vec!["project_id:project-a".to_owned()]
    );
}

#[rstest]
#[case::no_project_id(r#"{"token":"tok"}"#, None)]
#[case::missing_row(r#"{"token":"tok","project_id":"project-a"}"#, None)]
#[case::no_budget_table(r#"{"token":"tok","project_id":"project-a"}"#, Some("null"))]
#[case::zero_budget(
    r#"{"token":"tok","project_id":"project-a"}"#,
    Some(r#"{"max_budget":0}"#)
)]
#[case::negative_budget(
    r#"{"token":"tok","project_id":"project-a"}"#,
    Some(r#"{"max_budget":-2}"#)
)]
#[tokio::test]
async fn project_skip_zero_or_missing_budget(#[case] token: &str, #[case] budget: Option<&str>) {
    let mut fixture = Fixture::new();
    fixture.token = auth(token);
    if let Some(spec) = budget {
        fixture
            .cache
            .put("project_id:project-a", &project_row(spec, "0"));
    }
    assert!(fixture.counters().await.is_empty());
}
