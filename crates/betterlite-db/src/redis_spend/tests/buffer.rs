use std::sync::Arc;

use rust_decimal::Decimal;

use crate::daily_spend_sql::values::SpendColumnValue;
use crate::queues::test_support::captured_logs;
use crate::redis_spend::buffer::{RedisSpendError, SpendBufferPipeline};
use crate::redis_spend::codec::decode_log_record;
use crate::redis_spend::events::{
    REDIS_BUFFER_EVENT_CALL_TYPE, RedisBufferEvent, RedisBufferEventSink,
};
use crate::redis_spend::keys::{
    REDIS_DAILY_AGENT_BUFFER_KEY, REDIS_DAILY_ORG_BUFFER_KEY, REDIS_DAILY_TAG_BUFFER_KEY,
    REDIS_DAILY_TEAM_BUFFER_KEY, REDIS_DAILY_USER_BUFFER_KEY, REDIS_END_USER_BUFFER_KEY,
    REDIS_SPEND_LOGS_BUFFER_KEY, REDIS_SPEND_LOGS_MAX_ROWS, REDIS_UPDATE_BUFFER_KEY,
    REDIS_WINDOW_BUFFER_KEY, SpendBufferQueue,
};
use crate::redis_spend::lists::BufferReply;
use crate::spend_queues::daily::DailySpendUpdate;
use crate::spend_queues::entity::{SpendAmount, SpendUpdateTransactions};
use crate::window_queue::WindowSpendTransaction;

use super::support::{
    FakeLists, RecordingEvents, attached, buffer_with_sink, bytes, decoded_row, detached,
};

fn amount(value: &str) -> SpendAmount {
    SpendAmount(value.parse::<Decimal>().unwrap())
}

fn dec(value: &str) -> Decimal {
    value.parse::<Decimal>().unwrap()
}

fn transactions(user: &[(&str, &str)]) -> SpendUpdateTransactions {
    SpendUpdateTransactions {
        user_list_transactions: user
            .iter()
            .map(|(id, value)| ((*id).to_owned(), amount(value)))
            .collect(),
        ..SpendUpdateTransactions::default()
    }
}

fn daily_payload(prompt_tokens: i64) -> Vec<u8> {
    format!(
        r#"{{"day-1":{{"spend":1.5,"prompt_tokens":{prompt_tokens},"completion_tokens":2,"api_requests":1,"successful_requests":1,"failed_requests":0}}}}"#
    )
    .into_bytes()
}

fn window_record() -> WindowSpendTransaction {
    WindowSpendTransaction {
        entity_type: "key".to_owned(),
        entity_id: "key-1".to_owned(),
        window_duration: "1h".to_owned(),
        window_start: "2024-01-01T00:00:00+00:00".to_owned(),
        spend: amount("2.5"),
        started_at: None,
    }
}

fn window_payload() -> Vec<u8> {
    br#"[{"entity_type":"key","entity_id":"key-1","window_duration":"1h","window_start":"2024-01-01T00:00:00+00:00","spend":2.5}]"#.to_vec()
}

#[tokio::test]
async fn the_pipeline_pops_the_seven_queues_in_order_with_the_configured_count() {
    let lists = Arc::new(FakeLists::default());
    lists.script_pipeline(vec![
        BufferReply::List(vec![bytes(r#"{"user_list_transactions":{"user-1":1.5}}"#)]),
        BufferReply::List(vec![daily_payload(1)]),
        BufferReply::Missing,
        BufferReply::Missing,
        BufferReply::Missing,
        BufferReply::Missing,
        BufferReply::List(vec![window_payload()]),
    ]);
    let pipeline = attached(&lists).drain_pipeline().await.unwrap();
    let piped = lists.piped();
    assert_eq!(piped.len(), 1);
    assert_eq!(
        piped[0]
            .iter()
            .map(|operation| operation.key.as_str())
            .collect::<Vec<_>>(),
        vec![
            REDIS_UPDATE_BUFFER_KEY,
            REDIS_DAILY_USER_BUFFER_KEY,
            REDIS_DAILY_TEAM_BUFFER_KEY,
            REDIS_DAILY_ORG_BUFFER_KEY,
            REDIS_END_USER_BUFFER_KEY,
            REDIS_DAILY_AGENT_BUFFER_KEY,
            REDIS_WINDOW_BUFFER_KEY,
        ]
    );
    assert!(piped[0].iter().all(|operation| operation.count == Some(5)));
    assert_eq!(pipeline.entity, Some(transactions(&[("user-1", "1.5")])));
    assert_eq!(
        pipeline.daily_user.unwrap()["day-1"]["spend"],
        SpendColumnValue::Decimal(dec("1.5"))
    );
    assert_eq!(pipeline.daily_team, None);
    assert_eq!(pipeline.window, Some(vec![window_record()]));
}

#[tokio::test]
async fn a_short_pipeline_reply_pads_the_missing_slots_with_none() {
    let lists = Arc::new(FakeLists::default());
    lists.script_pipeline(vec![BufferReply::List(vec![bytes(
        r#"{"user_list_transactions":{"user-1":1.5}}"#,
    )])]);
    let pipeline = attached(&lists).drain_pipeline().await.unwrap();
    assert_eq!(pipeline.entity, Some(transactions(&[("user-1", "1.5")])));
    assert_eq!(pipeline.daily_user, None);
    assert_eq!(pipeline.daily_team, None);
    assert_eq!(pipeline.daily_org, None);
    assert_eq!(pipeline.daily_end_user, None);
    assert_eq!(pipeline.daily_agent, None);
    assert_eq!(pipeline.window, None);
}

#[tokio::test]
async fn extra_pipeline_replies_are_ignored() {
    let lists = Arc::new(FakeLists::default());
    let mut replies = vec![BufferReply::Missing; 7];
    replies.push(BufferReply::List(vec![bytes("{not json")]));
    replies.push(BufferReply::Scalar(bytes("x")));
    lists.script_pipeline(replies);
    let pipeline = attached(&lists).drain_pipeline().await.unwrap();
    assert_eq!(pipeline, SpendBufferPipeline::default());
}

#[tokio::test]
async fn an_entity_scalar_reply_is_one_batch() {
    let lists = Arc::new(FakeLists::default());
    lists.script_pipeline(vec![BufferReply::Scalar(bytes(
        r#"{"user_list_transactions":{"user-1":2.5}}"#,
    ))]);
    let pipeline = attached(&lists).drain_pipeline().await.unwrap();
    assert_eq!(pipeline.entity, Some(transactions(&[("user-1", "2.5")])));
}

#[tokio::test]
async fn a_daily_scalar_reply_is_an_error() {
    let lists = Arc::new(FakeLists::default());
    let mut replies = vec![BufferReply::Missing; 7];
    replies[1] = BufferReply::Scalar(daily_payload(1));
    lists.script_pipeline(replies);
    let error = attached(&lists).drain_pipeline().await.unwrap_err();
    assert!(matches!(error, RedisSpendError::ScalarReply));
}

#[tokio::test]
async fn a_window_scalar_reply_is_an_error() {
    let lists = Arc::new(FakeLists::default());
    let mut replies = vec![BufferReply::Missing; 7];
    replies[6] = BufferReply::Scalar(window_payload());
    lists.script_pipeline(replies);
    let error = attached(&lists).drain_pipeline().await.unwrap_err();
    assert!(matches!(error, RedisSpendError::ScalarReply));
}

#[tokio::test]
async fn a_daily_slot_with_an_empty_list_is_an_empty_aggregate() {
    let lists = Arc::new(FakeLists::default());
    let mut replies = vec![BufferReply::Missing; 7];
    replies[1] = BufferReply::List(Vec::new());
    lists.script_pipeline(replies);
    let pipeline = attached(&lists).drain_pipeline().await.unwrap();
    assert_eq!(pipeline.daily_user, Some(DailySpendUpdate::new()));
}

#[tokio::test]
async fn a_decode_error_after_the_pop_propagates() {
    let lists = Arc::new(FakeLists::default());
    lists.script_pipeline(vec![BufferReply::List(vec![bytes("{not json")])]);
    let error = attached(&lists).drain_pipeline().await.unwrap_err();
    assert!(matches!(error, RedisSpendError::Decode(_)));
    assert_eq!(
        lists.piped().len(),
        1,
        "the pop must happen before the parse"
    );
}

#[tokio::test]
async fn a_list_failure_propagates_as_a_redis_error() {
    let lists = Arc::new(FakeLists::default());
    lists.fail_reads();
    assert!(matches!(
        attached(&lists).drain_pipeline().await.unwrap_err(),
        RedisSpendError::List(_)
    ));
    assert!(matches!(
        attached(&lists).drain_spend_logs(10).await.unwrap_err(),
        RedisSpendError::List(_)
    ));
}

#[tokio::test]
async fn the_daily_tag_reader_pops_its_own_queue_with_the_configured_count() {
    let lists = Arc::new(FakeLists::default());
    lists.script_pop(BufferReply::List(vec![daily_payload(1), daily_payload(2)]));
    let mut buffer = attached(&lists);
    buffer.dequeue_count = 2;
    let tags = buffer.drain_daily_tags().await.unwrap().unwrap();
    assert_eq!(
        lists.popped(),
        vec![(REDIS_DAILY_TAG_BUFFER_KEY.to_owned(), Some(2))]
    );
    assert_eq!(tags["day-1"]["prompt_tokens"], SpendColumnValue::Int(3));
    assert_eq!(tags["day-1"]["spend"], SpendColumnValue::Decimal(dec("3")));
}

#[tokio::test]
async fn the_daily_tag_reader_keeps_a_missing_queue_as_none() {
    let lists = Arc::new(FakeLists::default());
    assert_eq!(attached(&lists).drain_daily_tags().await.unwrap(), None);
}

#[tokio::test]
async fn a_daily_tag_scalar_reply_is_an_error() {
    let lists = Arc::new(FakeLists::default());
    lists.script_pop(BufferReply::Scalar(daily_payload(1)));
    let error = attached(&lists).drain_daily_tags().await.unwrap_err();
    assert!(matches!(error, RedisSpendError::ScalarReply));
}

#[tokio::test]
async fn parking_needs_rows_an_enabled_buffer_and_attached_lists() {
    let lists = Arc::new(FakeLists::default());
    let row = decoded_row(r#"{"spend":1.5}"#);
    let mut disabled = attached(&lists);
    disabled.enabled = false;
    assert!(!disabled.park_spend_logs(std::slice::from_ref(&row)).await);
    assert!(!attached(&lists).park_spend_logs(&[]).await);
    assert!(!detached().park_spend_logs(std::slice::from_ref(&row)).await);
    assert!(lists.pushed().is_empty());
}

#[tokio::test]
async fn parking_pushes_the_encoded_rows_to_the_capped_key() {
    let lists = Arc::new(FakeLists::default());
    let row = decoded_row(r#"{"spend":1.5,"messages":"a\u0000b"}"#);
    assert!(
        attached(&lists)
            .park_spend_logs(std::slice::from_ref(&row))
            .await
    );
    let pushed = lists.pushed();
    assert_eq!(pushed.len(), 1);
    assert_eq!(pushed[0].key, REDIS_SPEND_LOGS_BUFFER_KEY);
    assert_eq!(pushed[0].max_len, REDIS_SPEND_LOGS_MAX_ROWS);
    assert!(
        pushed[0].values[0].contains(r#"\u0000"#),
        "the encoder stripped the nul: {}",
        pushed[0].values[0]
    );
    let reread = decode_log_record(pushed[0].values[0].as_bytes())
        .unwrap()
        .unwrap();
    assert_eq!(reread, row);
}

#[tokio::test]
async fn parking_logs_the_number_of_oldest_rows_the_cap_dropped() {
    let (logs, _guard) = captured_logs();
    let lists = Arc::new(FakeLists::default());
    lists.trim_size(REDIS_SPEND_LOGS_MAX_ROWS.saturating_add(3));
    let row = decoded_row(r#"{"spend":1.5}"#);
    assert!(
        attached(&lists)
            .park_spend_logs(std::slice::from_ref(&row))
            .await
    );
    assert!(logs.text().contains("dropped the 3 oldest spend logs"));
}

#[tokio::test]
async fn parking_at_the_cap_logs_no_drop() {
    let (logs, _guard) = captured_logs();
    let lists = Arc::new(FakeLists::default());
    lists.trim_size(REDIS_SPEND_LOGS_MAX_ROWS);
    let row = decoded_row(r#"{"spend":1.5}"#);
    assert!(
        attached(&lists)
            .park_spend_logs(std::slice::from_ref(&row))
            .await
    );
    assert!(
        !logs.text().contains("row cap"),
        "a push that stays within the cap must not warn: {}",
        logs.text()
    );
}

#[tokio::test]
async fn a_trim_failure_keeps_the_rows_in_memory() {
    let (logs, _guard) = captured_logs();
    let lists = Arc::new(FakeLists::default());
    lists.fail_trim();
    let row = decoded_row(r#"{"spend":1.5}"#);
    assert!(
        !attached(&lists)
            .park_spend_logs(std::slice::from_ref(&row))
            .await
    );
    assert!(
        logs.text()
            .contains("failed to park 1 spend log rows in Redis")
    );
}

#[tokio::test]
async fn draining_logs_takes_a_scalar_reply_and_drops_a_nonobject_row() {
    let lists = Arc::new(FakeLists::default());
    lists.script_pop(BufferReply::Scalar(bytes(r#"{"spend":1.5}"#)));
    lists.script_pop(BufferReply::List(vec![
        bytes("[1,2]"),
        bytes(r#"{"total_tokens":7}"#),
    ]));
    let buffer = attached(&lists);
    assert_eq!(
        buffer.drain_spend_logs(10).await.unwrap(),
        vec![decoded_row(r#"{"spend":1.5}"#)]
    );
    assert_eq!(
        buffer.drain_spend_logs(10).await.unwrap(),
        vec![decoded_row(r#"{"total_tokens":7}"#)]
    );
    assert_eq!(
        lists.popped(),
        vec![
            (REDIS_SPEND_LOGS_BUFFER_KEY.to_owned(), Some(10)),
            (REDIS_SPEND_LOGS_BUFFER_KEY.to_owned(), Some(10)),
        ]
    );
}

#[tokio::test]
async fn draining_logs_propagates_malformed_json() {
    let lists = Arc::new(FakeLists::default());
    lists.script_pop(BufferReply::List(vec![bytes("{not json")]));
    assert!(matches!(
        attached(&lists).drain_spend_logs(10).await.unwrap_err(),
        RedisSpendError::Decode(_)
    ));
}

#[tokio::test]
async fn draining_logs_is_empty_when_disabled_or_detached() {
    let lists = Arc::new(FakeLists::default());
    let mut disabled = attached(&lists);
    disabled.enabled = false;
    assert_eq!(disabled.drain_spend_logs(10).await.unwrap(), Vec::new());
    assert_eq!(detached().drain_spend_logs(10).await.unwrap(), Vec::new());
    assert!(lists.popped().is_empty());
}

#[tokio::test]
async fn a_detached_buffer_drains_nothing() {
    let buffer = detached();
    assert_eq!(
        buffer.drain_pipeline().await.unwrap(),
        SpendBufferPipeline::default()
    );
    assert_eq!(buffer.drain_daily_tags().await.unwrap(), None);
}

#[test]
fn the_queue_size_event_keeps_the_source_label_and_gauge_values() {
    let events = Arc::new(RecordingEvents::default());
    let sink: Arc<dyn RedisBufferEventSink> = Arc::clone(&events);
    let buffer = buffer_with_sink(sink);
    buffer.events.emit_queue_size(SpendBufferQueue::Window, 12);
    assert_eq!(
        events.events(),
        vec![RedisBufferEvent {
            queue: SpendBufferQueue::Window,
            queue_size: 12,
        }]
    );
    assert_eq!(
        REDIS_BUFFER_EVENT_CALL_TYPE,
        "_emit_new_item_added_to_queue_event"
    );
    assert_eq!(
        SpendBufferQueue::Window.service(),
        "redis_window_spend_update_queue"
    );
    assert_eq!(
        SpendBufferQueue::DailyTag.service(),
        "redis_daily_tag_spend_update_queue"
    );
}
