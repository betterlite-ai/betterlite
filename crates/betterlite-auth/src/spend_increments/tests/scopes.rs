use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::key_user_writes::hash::hash_token;
use rstest::rstest;
use rust_decimal::Decimal;

use crate::spend_counter_batch::batch::SpendCounterBatchError;
use crate::spend_increments::error::SpendIncrementError;
use crate::spend_increments::increment_spend_counters;
use crate::spend_writes::error::SpendWriteError;

use super::support::{Fixture, applied_adjustment, decimal, finalized, handle, input, texts};

#[tokio::test]
async fn every_entity_scope_charges_its_litellm_counter_key() {
    let fixture = Fixture::new(false);
    let mut request = input(Some("tok"));
    request.team_id = Some("t1");
    request.user_id = Some("u1");
    request.org_id = Some("o1");
    request.end_user_id = Some("eu1");
    request.project_id = Some("p1");

    increment_spend_counters(&request, &fixture.services())
        .await
        .unwrap();

    assert_eq!(fixture.counter("spend:key:tok").await, Some(decimal("2")));
    assert_eq!(fixture.counter("spend:team:t1").await, Some(decimal("2")));
    assert_eq!(
        fixture.counter("spend:team_member:u1:t1").await,
        Some(decimal("2"))
    );
    assert_eq!(fixture.counter("spend:user:u1").await, Some(decimal("2")));
    assert_eq!(
        fixture.counter("spend:end_user:eu1").await,
        Some(decimal("2"))
    );
    assert_eq!(fixture.counter("spend:org:o1").await, Some(decimal("2")));
    assert_eq!(
        fixture.counter("spend:project:p1").await,
        Some(decimal("2"))
    );
}

#[tokio::test]
async fn tags_and_model_access_groups_charge_once_each() {
    let fixture = Fixture::new(false);
    let mut tags = texts(&["a", "b", "a", ""]);
    tags.push(JsonData::Null);
    let mut groups = texts(&["g1", "g1", ""]);
    groups.push(JsonData::Bool(true));
    let mut request = input(Some("tok"));
    request.tags = Some(&tags);
    request.model_access_groups = Some(&groups);

    increment_spend_counters(&request, &fixture.services())
        .await
        .unwrap();

    assert_eq!(fixture.counter("spend:tag:a").await, Some(decimal("2")));
    assert_eq!(fixture.counter("spend:tag:b").await, Some(decimal("2")));
    assert_eq!(fixture.counter("spend:tag:").await, None);
    assert_eq!(
        fixture.counter("spend:model_access_group:g1").await,
        Some(decimal("2"))
    );
    assert_eq!(fixture.counter("spend:model_access_group:").await, None);
    let tag_reads = fixture
        .sources
        .reads()
        .into_iter()
        .filter(|cache_key| cache_key.as_str() == "tag:a")
        .count();
    assert_eq!(tag_reads, 1);
}

#[rstest]
#[case::raw_sk_token("sk-raw")]
#[case::stored_token("tok")]
#[tokio::test]
async fn a_raw_sk_token_is_hashed_once_and_a_stored_token_is_not(#[case] token: &str) {
    let fixture = Fixture::new(false);
    let mut request = input(Some(token));
    request.user_id = Some("u1");

    increment_spend_counters(&request, &fixture.services())
        .await
        .unwrap();

    let stored = if token.starts_with("sk-") {
        hash_token(token)
    } else {
        token.to_owned()
    };
    assert_eq!(
        fixture.counter(&format!("spend:key:{stored}")).await,
        Some(decimal("2"))
    );
    if token != stored {
        assert_eq!(fixture.counter(&format!("spend:key:{token}")).await, None);
    }
}

#[tokio::test]
async fn the_identity_scopes_read_their_source_cache_keys() {
    let fixture = Fixture::new(false);
    fixture.sources.put("tok", r#"{"spend": 1}"#);
    fixture.sources.put("team_id:t1", r#"{"spend": 1}"#);
    fixture
        .sources
        .put("team_membership:u1:t1", r#"{"spend": 1}"#);
    fixture.sources.put("u1", r#"{"spend": 1}"#);
    let mut request = input(Some("tok"));
    request.team_id = Some("t1");
    request.user_id = Some("u1");

    increment_spend_counters(&request, &fixture.services())
        .await
        .unwrap();

    assert_eq!(fixture.counter("spend:key:tok").await, Some(decimal("3")));
    assert_eq!(fixture.counter("spend:team:t1").await, Some(decimal("3")));
    assert_eq!(
        fixture.counter("spend:team_member:u1:t1").await,
        Some(decimal("3"))
    );
    assert_eq!(fixture.counter("spend:user:u1").await, Some(decimal("3")));
}

#[tokio::test]
async fn the_metering_scopes_read_their_source_cache_keys() {
    let fixture = Fixture::new(false);
    fixture.sources.put("end_user_id:eu1", r#"{"spend": 1}"#);
    fixture.sources.put("tag:a", r#"{"spend": 1}"#);
    fixture
        .sources
        .put("model_access_group:g1", r#"{"spend": 1}"#);
    fixture
        .sources
        .put("org_id:o1:with_budget", r#"{"spend": 1}"#);
    fixture.sources.put("project_id:p1", r#"{"spend": 1}"#);
    let tags = texts(&["a"]);
    let groups = texts(&["g1"]);
    let mut request = input(None);
    request.end_user_id = Some("eu1");
    request.tags = Some(&tags);
    request.model_access_groups = Some(&groups);
    request.org_id = Some("o1");
    request.project_id = Some("p1");

    increment_spend_counters(&request, &fixture.services())
        .await
        .unwrap();

    assert_eq!(
        fixture.counter("spend:end_user:eu1").await,
        Some(decimal("3"))
    );
    assert_eq!(fixture.counter("spend:tag:a").await, Some(decimal("3")));
    assert_eq!(
        fixture.counter("spend:model_access_group:g1").await,
        Some(decimal("3"))
    );
    assert_eq!(fixture.counter("spend:org:o1").await, Some(decimal("3")));
    assert_eq!(
        fixture.counter("spend:project:p1").await,
        Some(decimal("3"))
    );
    assert!(!fixture.sources.reads().iter().any(|key| key == "org_id:o1"));
}

#[tokio::test]
async fn the_plain_org_row_is_the_org_source_fallback() {
    let fixture = Fixture::new(false);
    fixture.sources.put("org_id:o1", r#"{"spend": 4}"#);
    let mut request = input(None);
    request.org_id = Some("o1");

    increment_spend_counters(&request, &fixture.services())
        .await
        .unwrap();

    assert_eq!(fixture.counter("spend:org:o1").await, Some(decimal("6")));
    assert_eq!(
        fixture.sources.reads(),
        vec!["org_id:o1:with_budget".to_owned(), "org_id:o1".to_owned()]
    );
}

#[tokio::test]
async fn a_user_without_a_team_and_a_project_without_an_org_charge_alone() {
    let fixture = Fixture::new(false);
    let mut request = input(None);
    request.user_id = Some("u1");
    request.project_id = Some("p1");

    increment_spend_counters(&request, &fixture.services())
        .await
        .unwrap();

    assert_eq!(fixture.counter("spend:user:u1").await, Some(decimal("2")));
    assert_eq!(
        fixture.counter("spend:project:p1").await,
        Some(decimal("2"))
    );
    assert_eq!(
        fixture.sources.reads(),
        vec!["u1".to_owned(), "project_id:p1".to_owned()]
    );
}

#[tokio::test]
async fn a_failing_item_leaves_its_siblings_charging() {
    let fixture = Fixture::new(false);
    fixture.batch.fail_for("spend:team_member:u1:t1");
    fixture.batch.fail_for("spend:tag:a");
    fixture.batch.fail_for("spend:model_access_group:g1");
    fixture.batch.fail_for("spend:org:o1");
    let tags = texts(&["a", "b"]);
    let groups = texts(&["g1", "g2"]);
    let mut request = input(None);
    request.team_id = Some("t1");
    request.user_id = Some("u1");
    request.end_user_id = Some("eu1");
    request.tags = Some(&tags);
    request.model_access_groups = Some(&groups);
    request.org_id = Some("o1");
    request.project_id = Some("p1");

    let result = increment_spend_counters(&request, &fixture.services()).await;

    assert!(matches!(
        result,
        Err(SpendIncrementError::Write(SpendWriteError::Batch(
            SpendCounterBatchError::Cancelled
        )))
    ));
    assert_eq!(fixture.counter("spend:team_member:u1:t1").await, None);
    assert_eq!(fixture.counter("spend:user:u1").await, Some(decimal("2")));
    assert_eq!(
        fixture.counter("spend:end_user:eu1").await,
        Some(decimal("2"))
    );
    assert_eq!(fixture.counter("spend:tag:a").await, None);
    assert_eq!(fixture.counter("spend:tag:b").await, Some(decimal("2")));
    assert_eq!(fixture.counter("spend:model_access_group:g1").await, None);
    assert_eq!(
        fixture.counter("spend:model_access_group:g2").await,
        Some(decimal("2"))
    );
    assert_eq!(fixture.counter("spend:org:o1").await, None);
    assert_eq!(
        fixture.counter("spend:project:p1").await,
        Some(decimal("2"))
    );
}

#[tokio::test]
async fn reserved_counters_are_skipped_by_the_direct_increment() {
    let fixture = Fixture::new(false);
    fixture.counters.reseed_unavailable();
    let reservation = handle("5", &["spend:key:tok", "spend:user:u1"]);
    let mut request = input(Some("tok"));
    request.team_id = Some("t1");
    request.user_id = Some("u1");
    request.budget_reservation = Some(&reservation);

    increment_spend_counters(&request, &fixture.services())
        .await
        .unwrap();

    assert_eq!(fixture.counter("spend:key:tok").await, None);
    assert_eq!(fixture.counter("spend:user:u1").await, None);
    assert_eq!(fixture.counter("spend:team:t1").await, Some(decimal("2")));
    assert!(fixture.counters.increments().is_empty());
    assert_eq!(applied_adjustment(&reservation), Some(decimal("-3")));
    assert!(finalized(&reservation));
}

#[tokio::test]
async fn a_finalized_reservation_charges_the_counters_directly() {
    let fixture = Fixture::new(false);
    let reservation = handle("5", &["spend:key:tok"]);
    betterlite_redis::request_batch::lock(&reservation.0).finalized = true;
    let mut request = input(Some("tok"));
    request.budget_reservation = Some(&reservation);

    increment_spend_counters(&request, &fixture.services())
        .await
        .unwrap();

    assert_eq!(fixture.counter("spend:key:tok").await, Some(decimal("2")));
    assert!(fixture.counters.increments().is_empty());
}

#[tokio::test]
async fn the_first_scope_error_is_returned_after_the_other_scopes_apply() {
    let fixture = Fixture::new(false);
    fixture
        .sources
        .put("tok", r#"{"budget_limits": "not json"}"#);
    fixture.batch.fail_for("spend:user:u1");
    let mut request = input(Some("tok"));
    request.team_id = Some("t1");
    request.user_id = Some("u1");

    let result = increment_spend_counters(&request, &fixture.services()).await;

    assert!(matches!(
        result,
        Err(SpendIncrementError::BudgetLimits { ref cache_key, .. })
            if cache_key.as_str() == "tok"
    ));
    assert_eq!(fixture.counter("spend:key:tok").await, None);
    assert_eq!(fixture.counter("spend:team:t1").await, Some(decimal("2")));
    assert_eq!(fixture.counter("spend:user:u1").await, None);
}

#[tokio::test]
async fn a_failed_key_prepare_skips_its_windows() {
    let fixture = Fixture::new(false);
    fixture.batch.fail_for("spend:key:tok");
    fixture.sources.put(
        "tok",
        r#"{"budget_limits": [{"budget_duration": "1d", "reset_at": "2026-11-18T00:00:00+00:00"}]}"#,
    );
    let mut request = input(Some("tok"));
    request.user_id = Some("u1");

    let result = increment_spend_counters(&request, &fixture.services()).await;

    assert!(matches!(
        result,
        Err(SpendIncrementError::Write(SpendWriteError::Batch(
            SpendCounterBatchError::Cancelled
        )))
    ));
    assert_eq!(fixture.counter("spend:key:tok").await, None);
    assert_eq!(fixture.counter("spend:key:tok:window:1d").await, None);
    assert!(fixture.queue.records().is_empty());
    assert_eq!(fixture.counter("spend:user:u1").await, Some(decimal("2")));
    assert_eq!(fixture.sources.reads(), vec!["u1".to_owned()]);
}

#[rstest]
#[case::zero_cost(Some(decimal("0")))]
#[case::absent_cost(None)]
#[tokio::test]
async fn a_zero_cost_without_a_reservation_charges_nothing(#[case] cost: Option<Decimal>) {
    let fixture = Fixture::new(false);
    let mut request = input(Some("tok"));
    request.user_id = Some("u1");
    request.response_cost = cost;

    increment_spend_counters(&request, &fixture.services())
        .await
        .unwrap();

    assert_eq!(fixture.counter("spend:key:tok").await, None);
    assert_eq!(fixture.counter("spend:user:u1").await, None);
    assert!(fixture.sources.reads().is_empty());
    assert!(fixture.queue.records().is_empty());
}
