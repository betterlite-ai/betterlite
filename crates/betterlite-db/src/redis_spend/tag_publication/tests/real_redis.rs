use betterlite_redis_fixture::server::RedisFixture;

use crate::redis_spend::keys::REDIS_DAILY_TAG_BUFFER_KEY;
use crate::redis_spend::tag_publication::store_in_memory_daily_tag_spend_updates_in_redis;
use crate::redis_spend::tests::real_redis::{buffer, cache_for, length, publisher, raw};
use crate::redis_spend::tests::support::{daily_queue, daily_row};

#[tokio::test]
async fn a_tag_cohort_lands_in_the_real_tag_list_and_the_reader_takes_it_back() {
    let fixture = RedisFixture::start().await.unwrap();
    let cache = cache_for(&fixture).await;
    let publisher = publisher(&cache);
    let queue = daily_queue(32, 16, 32);
    queue
        .add_update(daily_row("day-1", "1.5", 1))
        .await
        .unwrap();
    store_in_memory_daily_tag_spend_updates_in_redis(&publisher, &queue)
        .await
        .unwrap();
    let mut connection = raw(&fixture).await;
    assert_eq!(length(&mut connection, REDIS_DAILY_TAG_BUFFER_KEY).await, 1);
    assert_eq!(
        buffer(&cache).drain_daily_tags().await.unwrap().unwrap(),
        daily_row("day-1", "1.5", 1)
    );
}
