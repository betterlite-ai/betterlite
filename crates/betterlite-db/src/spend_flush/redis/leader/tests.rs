#![cfg(test)]

use std::sync::Arc;
use std::time::Duration;

use betterlite_redis_fixture::server::RedisFixture;
use tokio::task::JoinSet;
use tokio::time::timeout;

use crate::gateway_request_tracking::test_support::captured_logs;
use crate::queues::pod_lock::owner::lease_key;
use crate::redis_spend::codec::{decode_daily_batch, decode_entity_batch};
use crate::redis_spend::keys::{
    REDIS_DAILY_TAG_BUFFER_KEY, REDIS_DAILY_USER_BUFFER_KEY, REDIS_UPDATE_BUFFER_KEY,
    REDIS_WINDOW_BUFFER_KEY,
};
use crate::redis_spend::lists::BufferReply;
use crate::search_tools::create::dump::safe_dumps;
use crate::spend_commit::policy::SpendFailure;
use crate::spend_commit::tables::SpendTable;
use crate::spend_flush::fakes::tests::{ChunkStep, Events};
use crate::spend_flush::redis::DB_SPEND_UPDATE_JOB_NAME;
use crate::spend_flush::redis::fakes::tests::{
    FlushCase, TableFailWriter, UnreachableLists, buffer_length, daily_row, daily_user_rows,
    entity_batch, leader_case, push, range, raw, window_payload,
};
use crate::spend_queues::entity::{EntityType, SpendUpdateTransactions};

// The three keys one contended tick leaves behind: the refused pod's cohort survives and the holder's daily cohort and lease are gone.
async fn assert_contended_keys(fixture: &RedisFixture) {
    assert_eq!(
        buffer_length(fixture, REDIS_UPDATE_BUFFER_KEY).await,
        1,
        "the refused pod's published cohort must survive for the holder's next tick"
    );
    assert_eq!(buffer_length(fixture, REDIS_DAILY_USER_BUFFER_KEY).await, 0);
    assert_eq!(
        buffer_length(fixture, &lease_key(DB_SPEND_UPDATE_JOB_NAME)).await,
        0
    );
}

// One pod whose entity commit fails the team-member table, over a Redis that holds a single seeded entity batch.
async fn entity_failure_case(failure: SpendFailure) -> (RedisFixture, FlushCase<TableFailWriter>) {
    let fixture = RedisFixture::start().await.unwrap();
    let events = Arc::new(Events::default());
    let writer = Arc::new(TableFailWriter::new(SpendTable::TeamMemberList, failure));
    let case = FlushCase::new(&fixture, events, writer, Vec::new()).await;
    let mut connection = raw(&fixture).await;
    push(
        &mut connection,
        REDIS_UPDATE_BUFFER_KEY,
        &[safe_dumps(&entity_batch()).unwrap()],
    )
    .await;
    (fixture, case)
}

// The one entity batch a failed tick put back on the buffer, decoded.
async fn restored_entity(fixture: &RedisFixture) -> SpendUpdateTransactions {
    let mut connection = raw(fixture).await;
    let restored = range(&mut connection, REDIS_UPDATE_BUFFER_KEY, 0, -1).await;
    assert_eq!(
        restored.len(),
        1,
        "the failed tick restores one entity batch"
    );
    decode_entity_batch(restored[0].as_bytes()).unwrap()
}

// The uncommitted daily rows a failed tick put back on the user buffer, and the lease it released.
async fn assert_daily_remainder(fixture: &RedisFixture) {
    let mut connection = raw(fixture).await;
    let restored = range(&mut connection, REDIS_DAILY_USER_BUFFER_KEY, 0, -1).await;
    assert_eq!(restored.len(), 1);
    let remaining = decode_daily_batch(restored[0].as_bytes()).unwrap().len();
    assert_eq!(
        remaining, 50,
        "the keys the failed second chunk kept are restored"
    );
    assert_eq!(
        buffer_length(fixture, &lease_key(DB_SPEND_UPDATE_JOB_NAME)).await,
        0
    );
}

#[tokio::test]
async fn only_the_lease_holder_drains_and_a_refused_pod_keeps_its_published_cohort() {
    let fixture = RedisFixture::start().await.unwrap();
    let holder = Arc::new(leader_case(&fixture, vec![ChunkStep::WaitThenCommit]).await);
    holder
        .queues
        .add_entity(EntityType::User, "u-holder", "1.5")
        .await;
    holder.queues.add_daily_user("2026-01-02", "1").await;
    let refused = leader_case(&fixture, Vec::new()).await;
    refused
        .queues
        .add_entity(EntityType::User, "u-refused", "1.25")
        .await;

    let mut tasks = JoinSet::new();
    let flushing = Arc::clone(&holder);
    tasks.spawn(async move { flushing.flush().await });
    timeout(Duration::from_secs(5), holder.daily.entered().notified())
        .await
        .unwrap();

    refused.flush().await.unwrap();
    holder.daily.release().notify_one();
    tasks.join_next().await.unwrap().unwrap().unwrap();

    let holder_events = holder.events.entries();
    assert_eq!(
        holder_events,
        vec![
            "entity user_list_transactions 1".to_owned(),
            "daily User 1 WaitThenCommit".to_owned(),
        ]
    );
    let refused_events = refused.events.entries();
    assert!(
        refused_events.is_empty(),
        "the refused pod drained: {refused_events:?}"
    );
    assert_contended_keys(&fixture).await;
}

#[tokio::test]
async fn a_partial_entity_commit_restores_only_the_uncommitted_tables() {
    let (fixture, case) = entity_failure_case(SpendFailure::Deadlock).await;
    let (logs, _guard) = captured_logs();

    case.flush().await.unwrap();

    assert!(
        logs.text()
            .contains("Spend tracking - committing spend updates from Redis to DB"),
        "the holder reports the counts of the aggregate it popped"
    );
    assert_eq!(
        case.entity_writer.written(),
        vec![
            SpendTable::UserList,
            SpendTable::EndUserList,
            SpendTable::KeyList,
            SpendTable::TeamList,
        ],
        "the source order stops at the table in flight"
    );
    let leftover = restored_entity(&fixture).await;
    assert!(leftover.user_list_transactions.is_empty());
    assert!(leftover.end_user_list_transactions.is_empty());
    assert!(leftover.key_list_transactions.is_empty());
    assert!(leftover.team_list_transactions.is_empty());
    assert!(
        leftover
            .team_member_list_transactions
            .contains_key("team_id::t1::user_id::u1"),
        "a requeue-safe failure keeps the table in flight"
    );
    assert!(leftover.org_list_transactions.contains_key("o 1"));
    assert!(leftover.project_list_transactions.contains_key("p1"));
    assert_eq!(
        buffer_length(&fixture, &lease_key(DB_SPEND_UPDATE_JOB_NAME)).await,
        0
    );
}

#[tokio::test]
async fn an_unsafe_entity_failure_drops_the_table_in_flight_from_the_restore() {
    let (fixture, case) = entity_failure_case(SpendFailure::Rejected).await;

    case.flush().await.unwrap();

    let leftover = restored_entity(&fixture).await;
    assert!(
        leftover.team_member_list_transactions.is_empty(),
        "an unsafe failure never sends the table in flight again"
    );
    assert!(leftover.org_list_transactions.contains_key("o 1"));
    assert!(leftover.project_list_transactions.contains_key("p1"));
    assert_eq!(
        buffer_length(&fixture, &lease_key(DB_SPEND_UPDATE_JOB_NAME)).await,
        0
    );
}

#[tokio::test]
async fn a_partial_daily_commit_restores_the_remaining_rows() {
    let fixture = RedisFixture::start().await.unwrap();
    let mut connection = raw(&fixture).await;
    push(
        &mut connection,
        REDIS_DAILY_USER_BUFFER_KEY,
        &[safe_dumps(&daily_user_rows(150)).unwrap()],
    )
    .await;
    let retrying = vec![
        ChunkStep::Commit,
        ChunkStep::Fail(SpendFailure::NotSent),
        ChunkStep::Fail(SpendFailure::NotSent),
        ChunkStep::Fail(SpendFailure::NotSent),
        ChunkStep::Fail(SpendFailure::NotSent),
    ];
    let case = leader_case(&fixture, retrying).await;

    case.flush().await.unwrap();

    let entries = case.events.entries();
    assert_eq!(
        entries.first().map(String::as_str),
        Some("daily User 100 Commit"),
        "the batch size commits the first hundred rows as one chunk"
    );
    let failed = entries
        .iter()
        .filter(|event| event.as_str() == "daily User 50 Fail(NotSent)")
        .count();
    assert_eq!(
        failed, 4,
        "three retries make four attempts of the chunk the commit kept"
    );
    assert_eq!(case.pause.delays().len(), 3);
    assert_daily_remainder(&fixture).await;
}

#[tokio::test]
async fn an_unsafe_window_failure_drops_the_cohort_and_releases_the_lease() {
    let fixture = RedisFixture::start().await.unwrap();
    let case = leader_case(&fixture, Vec::new()).await;
    case.window.fail_with(SpendFailure::Ambiguous);
    let mut connection = raw(&fixture).await;
    push(
        &mut connection,
        REDIS_WINDOW_BUFFER_KEY,
        &[window_payload()],
    )
    .await;
    let (logs, _guard) = captured_logs();

    case.flush().await.unwrap();

    assert!(logs.text().contains("dropped 1 budget window increments"));
    assert_eq!(
        buffer_length(&fixture, REDIS_WINDOW_BUFFER_KEY).await,
        0,
        "an ambiguous window failure must not be sent again"
    );
    assert_eq!(
        buffer_length(&fixture, &lease_key(DB_SPEND_UPDATE_JOB_NAME)).await,
        0
    );
}

#[tokio::test]
async fn a_restore_that_cannot_reach_redis_logs_the_loss_and_releases_the_lease() {
    let fixture = RedisFixture::start().await.unwrap();
    let lists = Arc::new(UnreachableLists::default());
    lists.script_pipeline(vec![BufferReply::List(vec![
        safe_dumps(&entity_batch()).unwrap().into_bytes(),
    ])]);
    let mut case = leader_case(&fixture, Vec::new()).await;
    case.entity_writer.fail_with(SpendFailure::Deadlock);
    case.use_lists(&lists);
    let (logs, _guard) = captured_logs();

    case.flush().await.unwrap();

    assert!(logs.text().contains("These spend updates are lost"));
    assert_eq!(
        lists.push_attempts(),
        2,
        "the publish attempt and the failed restore are the only pushes"
    );
    assert_eq!(
        buffer_length(&fixture, &lease_key(DB_SPEND_UPDATE_JOB_NAME)).await,
        0,
        "a restore that cannot reach redis must not keep the lease"
    );
}

#[tokio::test]
async fn a_drain_error_releases_the_lease_without_restoring_anything() {
    let fixture = RedisFixture::start().await.unwrap();
    let case = leader_case(&fixture, Vec::new()).await;
    let mut connection = raw(&fixture).await;
    push(
        &mut connection,
        REDIS_UPDATE_BUFFER_KEY,
        &["{not json".to_owned()],
    )
    .await;
    let (logs, _guard) = captured_logs();

    case.flush().await.unwrap();

    assert!(
        logs.text()
            .contains("failed to commit spend updates from Redis to DB")
    );
    assert_eq!(buffer_length(&fixture, REDIS_UPDATE_BUFFER_KEY).await, 0);
    assert_eq!(
        buffer_length(&fixture, &lease_key(DB_SPEND_UPDATE_JOB_NAME)).await,
        0
    );
}

#[tokio::test]
async fn the_handlers_retry_count_reaches_the_redis_daily_commit() {
    let fixture = RedisFixture::start().await.unwrap();
    let case = leader_case(&fixture, vec![ChunkStep::Fail(SpendFailure::NotSent); 4]).await;
    case.queues.add_daily_user("2026-01-02", "1").await;

    case.flush().await.unwrap();

    let events = case.events.entries();
    let attempts = events
        .iter()
        .filter(|event| event.starts_with("daily User"))
        .count();
    assert_eq!(attempts, 4, "three retries make four attempts");
    assert_eq!(case.pause.delays().len(), 3);
    assert_eq!(
        buffer_length(&fixture, REDIS_DAILY_USER_BUFFER_KEY).await,
        1,
        "the whole map goes back after an exhausted retryable failure"
    );
}

#[tokio::test]
async fn the_main_flusher_leaves_the_daily_tag_cohort_in_redis() {
    let fixture = RedisFixture::start().await.unwrap();
    let case = leader_case(&fixture, Vec::new()).await;
    let mut connection = raw(&fixture).await;
    push(
        &mut connection,
        REDIS_DAILY_TAG_BUFFER_KEY,
        &[safe_dumps(&daily_row("day-1", "1.5", 1)).unwrap()],
    )
    .await;

    case.flush().await.unwrap();

    assert_eq!(
        buffer_length(&fixture, REDIS_DAILY_TAG_BUFFER_KEY).await,
        1,
        "the tag cohort belongs to its own scheduler job"
    );
}

#[tokio::test]
async fn the_handlers_retry_count_reaches_the_redis_entity_commit() {
    let fixture = RedisFixture::start().await.unwrap();
    let case = leader_case(&fixture, Vec::new()).await;
    case.entity_writer.fail_with(SpendFailure::Deadlock);
    case.queues
        .add_entity(EntityType::User, "u-retry", "2")
        .await;

    case.flush().await.unwrap();

    let attempts = case
        .events
        .entries()
        .iter()
        .filter(|event| event.starts_with("entity user_list_transactions"))
        .count();
    assert_eq!(attempts, 4, "three retries make four entity attempts");
    assert_eq!(case.pause.delays().len(), 3);
}
