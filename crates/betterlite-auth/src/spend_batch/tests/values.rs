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
    assert_eq!(
        read(&batch, "spend:key:false").await,
        Some(SpendCounterValue::Amount(decimal("0")))
    );
    assert_eq!(
        read(&batch, "spend:key:null").await,
        Some(SpendCounterValue::Missing)
    );
    assert_eq!(
        read(&batch, "spend:key:number").await,
        Some(SpendCounterValue::Amount(decimal("2.5")))
    );
    assert_eq!(
        read(&batch, "spend:key:precision").await,
        Some(SpendCounterValue::Amount(decimal("9.99")))
    );
    assert_eq!(
        read(&batch, "spend:key:scientific").await,
        Some(SpendCounterValue::Amount(decimal("150")))
    );
    assert_eq!(
        read(&batch, "spend:key:text").await,
        Some(SpendCounterValue::Amount(decimal("1.25")))
    );
    assert_eq!(
        read(&batch, "spend:key:true").await,
        Some(SpendCounterValue::Amount(decimal("1")))
    );
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
