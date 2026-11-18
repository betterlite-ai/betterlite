use std::sync::Arc;

use rstest::rstest;

use crate::spend_batch::batch::{SpendCounterBatch, SpendCounterValue};

use super::support::{DebugEvents, FakeRedis, Reply, allow_cache, decimal, keys};

async fn open_batch(server: &FakeRedis) -> Arc<SpendCounterBatch> {
    SpendCounterBatch::new(allow_cache(server).await)
}

async fn read(batch: &Arc<SpendCounterBatch>, counter_key: &str) -> Option<SpendCounterValue> {
    batch.read(counter_key).await.expect("the read succeeds")
}

async fn assert_amount(batch: &Arc<SpendCounterBatch>, counter_key: &str, amount: &str) {
    assert_eq!(
        read(batch, counter_key).await,
        Some(SpendCounterValue::Amount(decimal(amount)))
    );
}

async fn assert_missing(batch: &Arc<SpendCounterBatch>, counter_key: &str) {
    assert_eq!(
        read(batch, counter_key).await,
        Some(SpendCounterValue::Missing)
    );
}

#[tokio::test]
async fn decodes_every_counter_shape_from_one_mget() {
    let server = FakeRedis::start().await;
    let batch = open_batch(&server).await;
    batch.bind(&keys(&[
        "spend:key:false",
        "spend:key:null",
        "spend:key:number",
        "spend:key:precision",
        "spend:key:scientific",
        "spend:key:text",
        "spend:key:true",
    ]));
    server.script_mget(Reply::Array(vec![
        Reply::Bulk("false".to_owned()),
        Reply::Bulk("null".to_owned()),
        Reply::Bulk("2.5".to_owned()),
        Reply::Bulk("{\"$serde_json::private::Number\":\"9.99\"}".to_owned()),
        Reply::Bulk("1.5e2".to_owned()),
        Reply::Bulk("\"1.25\"".to_owned()),
        Reply::Bulk("true".to_owned()),
    ]));
    assert_amount(&batch, "spend:key:false", "0").await;
    assert_missing(&batch, "spend:key:null").await;
    assert_amount(&batch, "spend:key:number", "2.5").await;
    assert_amount(&batch, "spend:key:precision", "9.99").await;
    assert_amount(&batch, "spend:key:scientific", "150").await;
    assert_amount(&batch, "spend:key:text", "1.25").await;
    assert_amount(&batch, "spend:key:true", "1").await;
    assert_eq!(server.mget_keys().len(), 1);
}

#[rstest]
#[case::not_a_number("\"not-a-number\"")]
#[case::not_finite("\"nan\"")]
#[case::past_decimal_range("\"1e400\"")]
#[case::beyond_decimal_max("1e300")]
#[case::foreign_object("{\"other\":1}")]
#[tokio::test]
async fn a_value_the_decoder_rejects_drops_the_whole_map(#[case] payload: &str) {
    let server = FakeRedis::start().await;
    let events = DebugEvents::capture();
    let batch = open_batch(&server).await;
    batch.bind(&keys(&["spend:key:a", "spend:key:b"]));
    server.script_mget(Reply::Array(vec![
        Reply::Bulk("2".to_owned()),
        Reply::Bulk(payload.to_owned()),
    ]));
    assert_eq!(read(&batch, "spend:key:a").await, None);
    assert_eq!(read(&batch, "spend:key:b").await, None);
    assert_eq!(server.mget_keys().len(), 1);
    assert!(events.count() >= 1);
}
