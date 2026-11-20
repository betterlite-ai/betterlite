use std::collections::BTreeMap;
use std::error::Error;
use std::sync::Arc;

use betterlite_redis::redis_cache::RedisCache;
use betterlite_redis::redis_cache::connect::{RedisCacheOptions, connect};
use betterlite_redis::redis_client_reuse::breaker::{BreakerAdmission, BreakerOpen};
use betterlite_redis::redis_client_reuse::breaker_gate::BreakerGate;
use betterlite_redis::redis_client_reuse::service_event::ServiceEvent;
use betterlite_redis::redis_client_reuse::service_hooks::ServiceHooks;
use betterlite_redis::redis_client_reuse::store::RedisClientStore;
use betterlite_redis::redis_client_reuse::transport::RedisCacheFamily;
use betterlite_redis::redis_connection_settings::model::settings::RedisOverrides;
use betterlite_redis::redis_live_client::LiveClientFactory;
use betterlite_redis_fixture::server::RedisFixture;
use redis::aio::MultiplexedConnection;
use redis::{Client, cmd};
use rust_decimal::Decimal;

use crate::daily_spend_sql::values::SpendColumnValue;
use crate::queues::test_support::captured_logs;
use crate::redis_spend::buffer::{RedisSpendError, RedisUpdateBuffer, SpendBufferPipeline};
use crate::redis_spend::events::NoRedisBufferEventSink;
use crate::redis_spend::keys::{
    REDIS_DAILY_TAG_BUFFER_KEY, REDIS_DAILY_USER_BUFFER_KEY, REDIS_SPEND_LOGS_BUFFER_KEY,
    REDIS_SPEND_LOGS_MAX_ROWS, REDIS_UPDATE_BUFFER_KEY, REDIS_WINDOW_BUFFER_KEY,
};
use crate::redis_spend::lists::{RedisSpendLists, SpendBufferLists};
use crate::redis_spend::log_row::{SpendLogRow, SpendLogValue};
use crate::spend_queues::entity::{SpendAmount, SpendUpdateTransactions};

use super::support::decoded_row;

const CACHE_TTL_SECONDS: i64 = 60;
const SEEDED_ROWS: i64 = 100_000;
const CAPPED_ROWS: i64 = 100_002;

struct NoHooks;

impl ServiceHooks for NoHooks {
    fn success(&self, _event: &ServiceEvent) {}

    fn failure(&self, _event: &ServiceEvent, _error: &(dyn Error + Send + Sync)) {}
}

struct OpenGate;

impl BreakerGate for OpenGate {
    fn enter(&self, _name: &'static str) -> Result<BreakerAdmission, BreakerOpen> {
        Ok(BreakerAdmission::from(0))
    }

    fn exit(&self, _admission: BreakerAdmission) {}

    fn record_failure(&self, _error: &(dyn Error + Send + Sync)) {}

    fn swallowed(&self, _admission: &BreakerAdmission, _error: &(dyn Error + Send + Sync)) {}
}

fn amount(value: &str) -> SpendAmount {
    SpendAmount(value.parse::<Decimal>().unwrap())
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

fn daily_payload(prompt_tokens: i64) -> String {
    format!(
        r#"{{"day-1":{{"spend":1.5,"prompt_tokens":{prompt_tokens},"completion_tokens":2,"api_requests":1,"successful_requests":1,"failed_requests":0}}}}"#
    )
}

fn numbered_row(index: i64) -> SpendLogRow {
    SpendLogRow::from([("index".to_owned(), SpendLogValue::Whole(index))])
}

async fn cache_for(fixture: &RedisFixture) -> Arc<RedisCache> {
    let url = serde_json::to_string(fixture.as_ref()).unwrap();
    let overrides: RedisOverrides = serde_json::from_str(&format!(r#"{{"url":{url}}}"#)).unwrap();
    let options = RedisCacheOptions {
        overrides,
        family: RedisCacheFamily::Standalone,
        namespace: None,
        default_ttl: CACHE_TTL_SECONDS,
        redis_flush_size: None,
        default_major_version: 7,
        factory: Arc::new(LiveClientFactory::new(None)),
        store: Arc::new(RedisClientStore::new()),
        hooks: Arc::new(NoHooks),
        gate: Arc::new(OpenGate),
    };
    let env = BTreeMap::<String, String>::new();
    Arc::new(connect(options, &env).await.unwrap())
}

fn buffer(cache: &Arc<RedisCache>) -> RedisUpdateBuffer {
    let lists = RedisSpendLists {
        cache: Arc::clone(cache),
    };
    let port: Arc<dyn SpendBufferLists> = Arc::new(lists);
    RedisUpdateBuffer {
        lists: Some(port),
        enabled: true,
        dequeue_count: 5,
        events: Arc::new(NoRedisBufferEventSink),
    }
}

async fn raw(fixture: &RedisFixture) -> MultiplexedConnection {
    Client::open(fixture.as_ref())
        .unwrap()
        .get_multiplexed_async_connection()
        .await
        .unwrap()
}

async fn push(connection: &mut MultiplexedConnection, key: &str, values: &[String]) {
    let mut command = cmd("RPUSH");
    command.arg(key);
    for value in values {
        command.arg(value.as_str());
    }
    let _length: i64 = command.query_async(connection).await.unwrap();
}

async fn length(connection: &mut MultiplexedConnection, key: &str) -> i64 {
    cmd("LLEN").arg(key).query_async(connection).await.unwrap()
}

async fn range(
    connection: &mut MultiplexedConnection,
    key: &str,
    start: i64,
    stop: i64,
) -> Vec<String> {
    cmd("LRANGE")
        .arg(key)
        .arg(start)
        .arg(stop)
        .query_async(connection)
        .await
        .unwrap()
}

async fn lengths(connection: &mut MultiplexedConnection, keys: &[&str]) -> Vec<i64> {
    let mut measured = Vec::with_capacity(keys.len());
    for key in keys {
        measured.push(length(connection, key).await);
    }
    measured
}

#[tokio::test]
async fn parking_then_draining_takes_the_rows_out_of_redis() {
    let fixture = RedisFixture::start().await.unwrap();
    let cache = cache_for(&fixture).await;
    let buffer = buffer(&cache);
    let first = decoded_row(r#"{"spend":1.5}"#);
    let second = decoded_row(r#"{"total_tokens":7}"#);
    assert!(
        buffer
            .park_spend_logs(&[first.clone(), second.clone()])
            .await
    );
    let mut connection = raw(&fixture).await;
    assert_eq!(
        length(&mut connection, REDIS_SPEND_LOGS_BUFFER_KEY).await,
        2
    );
    assert_eq!(
        buffer.drain_spend_logs(10).await.unwrap(),
        vec![first, second]
    );
    assert_eq!(
        length(&mut connection, REDIS_SPEND_LOGS_BUFFER_KEY).await,
        0
    );
    assert!(buffer.drain_spend_logs(10).await.unwrap().is_empty());
}

#[tokio::test]
async fn parking_past_the_cap_keeps_the_newest_rows_and_logs_the_drop() {
    let (logs, _guard) = captured_logs();
    let fixture = RedisFixture::start().await.unwrap();
    let cache = cache_for(&fixture).await;
    let buffer = buffer(&cache);
    let seeded: Vec<SpendLogRow> = (0..SEEDED_ROWS).map(numbered_row).collect();
    assert!(buffer.park_spend_logs(&seeded).await);
    let extra: Vec<SpendLogRow> = (SEEDED_ROWS..CAPPED_ROWS).map(numbered_row).collect();
    assert!(buffer.park_spend_logs(&extra).await);
    let mut connection = raw(&fixture).await;
    assert_eq!(
        length(&mut connection, REDIS_SPEND_LOGS_BUFFER_KEY).await,
        REDIS_SPEND_LOGS_MAX_ROWS
    );
    let oldest = range(&mut connection, REDIS_SPEND_LOGS_BUFFER_KEY, 0, 0).await;
    assert!(
        oldest[0].contains("\"index\":2"),
        "the cap dropped the wrong rows: {}",
        oldest[0]
    );
    let newest = range(&mut connection, REDIS_SPEND_LOGS_BUFFER_KEY, -1, -1).await;
    assert!(newest[0].contains("\"index\":100001"));
    assert!(logs.text().contains("dropped the 2 oldest spend logs"));
}

#[tokio::test]
async fn the_pipeline_pops_every_queue_and_leaves_the_daily_tag_queue() {
    let fixture = RedisFixture::start().await.unwrap();
    let cache = cache_for(&fixture).await;
    let buffer = buffer(&cache);
    let mut connection = raw(&fixture).await;
    push(
        &mut connection,
        REDIS_UPDATE_BUFFER_KEY,
        &[r#"{"user_list_transactions":{"user-1":1.5}}"#.to_owned()],
    )
    .await;
    push(
        &mut connection,
        REDIS_DAILY_USER_BUFFER_KEY,
        &[daily_payload(1)],
    )
    .await;
    push(
        &mut connection,
        REDIS_WINDOW_BUFFER_KEY,
        &[r#"[{"entity_type":"key","entity_id":"key-1","window_duration":"1h","window_start":"2024-01-01T00:00:00+00:00","spend":2.5}]"#.to_owned()],
    )
    .await;
    push(
        &mut connection,
        REDIS_DAILY_TAG_BUFFER_KEY,
        &[daily_payload(1)],
    )
    .await;
    let pipeline = buffer.drain_pipeline().await.unwrap();
    assert_eq!(pipeline.entity, Some(transactions(&[("user-1", "1.5")])));
    let window = pipeline.window.unwrap();
    assert_eq!(window.len(), 1);
    assert_eq!(window[0].spend, amount("2.5"));
    assert_eq!(
        lengths(
            &mut connection,
            &[
                REDIS_UPDATE_BUFFER_KEY,
                REDIS_DAILY_USER_BUFFER_KEY,
                REDIS_WINDOW_BUFFER_KEY,
            ]
        )
        .await,
        vec![0, 0, 0]
    );
    assert_eq!(
        length(&mut connection, REDIS_DAILY_TAG_BUFFER_KEY).await,
        1,
        "the seven-slot pipeline must leave daily tags alone"
    );
}

#[tokio::test]
async fn a_malformed_entity_record_fails_after_its_pop() {
    let fixture = RedisFixture::start().await.unwrap();
    let cache = cache_for(&fixture).await;
    let buffer = buffer(&cache);
    let mut connection = raw(&fixture).await;
    push(
        &mut connection,
        REDIS_UPDATE_BUFFER_KEY,
        &["{not json".to_owned()],
    )
    .await;
    let error = buffer.drain_pipeline().await.unwrap_err();
    assert!(matches!(error, RedisSpendError::Decode(_)));
    assert_eq!(length(&mut connection, REDIS_UPDATE_BUFFER_KEY).await, 0);
}

#[tokio::test]
async fn the_daily_tag_reader_pops_only_the_configured_count() {
    let fixture = RedisFixture::start().await.unwrap();
    let cache = cache_for(&fixture).await;
    let mut buffer = buffer(&cache);
    buffer.dequeue_count = 2;
    let mut connection = raw(&fixture).await;
    push(
        &mut connection,
        REDIS_DAILY_TAG_BUFFER_KEY,
        &[daily_payload(1), daily_payload(2), daily_payload(3)],
    )
    .await;
    let tags = buffer.drain_daily_tags().await.unwrap().unwrap();
    assert_eq!(tags["day-1"]["prompt_tokens"], SpendColumnValue::Int(3));
    assert_eq!(length(&mut connection, REDIS_DAILY_TAG_BUFFER_KEY).await, 1);
}

#[tokio::test]
async fn an_empty_buffer_drains_to_nothing() {
    let fixture = RedisFixture::start().await.unwrap();
    let cache = cache_for(&fixture).await;
    let buffer = buffer(&cache);
    assert_eq!(
        buffer.drain_pipeline().await.unwrap(),
        SpendBufferPipeline::default()
    );
    assert_eq!(buffer.drain_daily_tags().await.unwrap(), None);
    assert!(buffer.drain_spend_logs(10).await.unwrap().is_empty());
}
