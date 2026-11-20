#![cfg(test)]
#![cfg_attr(
    test,
    expect(
        clippy::too_many_lines,
        reason = "each test reads top to bottom as the failed commit it describes: build the cohorts, restore them, then assert the pipeline and the log"
    )
)]

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::mcp_stored_contracts::MAX_STORED_BYTES;
use crate::redis_spend::codec::{decode_daily_batch, decode_entity_batch, decode_window_batch};
use crate::redis_spend::keys::{
    REDIS_DAILY_TAG_BUFFER_KEY, REDIS_DAILY_TEAM_BUFFER_KEY, REDIS_UPDATE_BUFFER_KEY,
    REDIS_WINDOW_BUFFER_KEY,
};
use crate::redis_spend::republication::{
    RepublicationError, UncommittedCohorts, restore_transactions_to_redis,
};
use crate::redis_spend::tests::support::{
    FakeLists, captured_logs, daily_row, detached_publisher, publisher, transactions,
    window_transaction,
};
use crate::window_queue::WindowSpendWirePayload;

#[tokio::test]
async fn without_a_redis_client_an_oversized_cohort_is_never_encoded() {
    let oversized = "x".repeat(MAX_STORED_BYTES + 1);
    let cohorts = UncommittedCohorts {
        daily_tag: Some(daily_row(&oversized, "1.5", 1)),
        ..UncommittedCohorts::default()
    };
    restore_transactions_to_redis(&detached_publisher(), cohorts)
        .await
        .unwrap();
}

#[tokio::test]
async fn falsey_cohorts_are_skipped_and_the_rest_go_out_in_restore_order() {
    let (logs, _guard) = captured_logs();
    let lists = Arc::new(FakeLists::default());
    let publisher = publisher(&lists);
    lists.script_push_lengths(vec![11, 3, 4, 5]);
    let cohorts = UncommittedCohorts {
        entity: Some(transactions(&[("user-1", "1.5")])),
        daily_user: Some(BTreeMap::new()),
        daily_team: Some(daily_row("day-2", "2.5", 2)),
        daily_tag: Some(daily_row("day-3", "3.5", 3)),
        window: Some(vec![window_transaction("key", "key-1", "4.5")]),
        ..UncommittedCohorts::default()
    };
    restore_transactions_to_redis(&publisher, cohorts)
        .await
        .unwrap();
    let pipelines = lists.pushed_pipelines();
    assert_eq!(pipelines.len(), 1);
    let operations = &pipelines[0];
    let keys: Vec<&str> = operations
        .iter()
        .map(|operation| operation.key.as_str())
        .collect();
    assert_eq!(
        keys,
        [
            REDIS_UPDATE_BUFFER_KEY,
            REDIS_DAILY_TEAM_BUFFER_KEY,
            REDIS_DAILY_TAG_BUFFER_KEY,
            REDIS_WINDOW_BUFFER_KEY,
        ],
        "an empty daily cohort is falsey and never reaches the pipeline"
    );
    assert_eq!(
        decode_entity_batch(operations[0].values[0].as_bytes()).unwrap(),
        transactions(&[("user-1", "1.5")])
    );
    assert_eq!(
        decode_daily_batch(operations[1].values[0].as_bytes()).unwrap(),
        daily_row("day-2", "2.5", 2)
    );
    assert_eq!(
        decode_daily_batch(operations[2].values[0].as_bytes()).unwrap(),
        daily_row("day-3", "3.5", 3)
    );
    let payloads: Vec<WindowSpendWirePayload> =
        serde_json::from_str(&operations[3].values[0]).unwrap();
    assert_eq!(payloads.len(), 1);
    assert!(
        payloads[0].request_ids.is_empty(),
        "a republished window tuple wears the wire payload's empty request_ids"
    );
    assert_eq!(
        decode_window_batch(operations[3].values[0].as_bytes()).unwrap(),
        vec![window_transaction("key", "key-1", "4.5")]
    );
    assert!(
        logs.text()
            .contains("restored 4 uncommitted transaction set(s) to Redis")
    );
}

#[tokio::test]
async fn empty_cohorts_send_no_pipeline() {
    let lists = Arc::new(FakeLists::default());
    let publisher = publisher(&lists);
    let cohorts = UncommittedCohorts {
        daily_tag: Some(BTreeMap::new()),
        window: Some(Vec::new()),
        ..UncommittedCohorts::default()
    };
    restore_transactions_to_redis(&publisher, cohorts)
        .await
        .unwrap();
    assert!(lists.pushed_pipelines().is_empty());
}

#[tokio::test]
async fn a_redis_failure_loses_the_cohorts_and_says_so() {
    let (logs, _guard) = captured_logs();
    let lists = Arc::new(FakeLists::default());
    let publisher = publisher(&lists);
    lists.fail_push();
    let cohorts = UncommittedCohorts {
        daily_tag: Some(daily_row("day-3", "3.5", 3)),
        ..UncommittedCohorts::default()
    };
    restore_transactions_to_redis(&publisher, cohorts)
        .await
        .unwrap();
    let text = logs.text();
    assert!(text.contains("failed to restore uncommitted transactions to Redis"));
    assert!(text.contains("These spend updates are lost"));
    assert!(!text.contains("restored 1 uncommitted transaction set(s)"));
}

#[tokio::test]
async fn an_encoding_failure_propagates_instead_of_being_logged_as_lost() {
    let lists = Arc::new(FakeLists::default());
    let publisher = publisher(&lists);
    let oversized = "x".repeat(MAX_STORED_BYTES + 1);
    let cohorts = UncommittedCohorts {
        daily_tag: Some(daily_row(&oversized, "1.5", 1)),
        ..UncommittedCohorts::default()
    };
    let error = restore_transactions_to_redis(&publisher, cohorts)
        .await
        .unwrap_err();
    assert!(matches!(error, RepublicationError::Encode(_)), "{error:?}");
    assert!(lists.pushed_pipelines().is_empty());
}
