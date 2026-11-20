use std::sync::Arc;

use betterlite_redis_fixture::server::RedisFixture;

use crate::daily_spend_sql::values::SpendColumnValue;
use crate::redis_spend::events::RedisBufferEventSink;
use crate::redis_spend::keys::{
    REDIS_DAILY_TEAM_BUFFER_KEY, REDIS_DAILY_USER_BUFFER_KEY, REDIS_UPDATE_BUFFER_KEY,
    SpendBufferQueue,
};
use crate::redis_spend::lists::{RedisSpendLists, SpendBufferPush};
use crate::redis_spend::publication::{
    RedisSpendPublisher, SpendStaging, SpendUpdateQueues, store_spend_updates_in_redis,
};
use crate::redis_spend::tests::real_redis::{buffer, cache_for, length, raw};
use crate::redis_spend::tests::support::{
    RecordingEvents, amount, daily_queue, daily_row, queue_item, transactions, window_queue,
    window_transaction,
};
use crate::spend_queues::entity::{EntityType, SpendUpdateQueue};
use crate::spend_queues::events::NoSpendQueueEventSink;

use super::gauge;

#[tokio::test]
async fn publication_appends_each_cohort_to_its_real_redis_list() {
    let fixture = RedisFixture::start().await.unwrap();
    let cache = cache_for(&fixture).await;
    let lists: Arc<dyn SpendBufferPush> = Arc::new(RedisSpendLists {
        cache: Arc::clone(&cache),
    });
    let events = Arc::new(RecordingEvents::default());
    let sink: Arc<dyn RedisBufferEventSink> = Arc::<RecordingEvents>::clone(&events);
    let publisher = RedisSpendPublisher {
        lists: Some(lists),
        events: sink,
        staging: SpendStaging::default(),
    };
    let entity = SpendUpdateQueue::new(32, 16, 32, Arc::new(NoSpendQueueEventSink)).unwrap();
    let daily_user = daily_queue(32, 16, 32);
    let daily_team = daily_queue(32, 16, 32);
    let daily_org = daily_queue(32, 16, 32);
    let daily_end_user = daily_queue(32, 16, 32);
    let daily_agent = daily_queue(32, 16, 32);
    let window = window_queue(32, 16, 32);
    entity
        .add_update(queue_item(EntityType::User, "user-1", "1.5"))
        .await
        .unwrap();
    daily_user
        .add_update(daily_row("day-1", "1.5", 1))
        .await
        .unwrap();
    window
        .add_update(window_transaction("key", "key-1", "3.5"))
        .await
        .unwrap();
    let queues = SpendUpdateQueues {
        entity: &entity,
        daily_user: &daily_user,
        daily_team: &daily_team,
        daily_org: &daily_org,
        daily_end_user: &daily_end_user,
        daily_agent: &daily_agent,
        window: Some(&window),
    };
    store_spend_updates_in_redis(&publisher, queues)
        .await
        .unwrap();
    let mut connection = raw(&fixture).await;
    assert_eq!(length(&mut connection, REDIS_UPDATE_BUFFER_KEY).await, 1);
    assert_eq!(
        length(&mut connection, REDIS_DAILY_USER_BUFFER_KEY).await,
        1
    );
    assert_eq!(
        length(&mut connection, REDIS_DAILY_TEAM_BUFFER_KEY).await,
        0
    );
    assert_eq!(
        events.events(),
        vec![
            gauge(SpendBufferQueue::Entity, 1),
            gauge(SpendBufferQueue::DailyUser, 1),
            gauge(SpendBufferQueue::Window, 1),
        ],
        "the gauge reports the length Redis returns after the append"
    );
    let drained = buffer(&cache).drain_pipeline().await.unwrap();
    assert_eq!(drained.entity, Some(transactions(&[("user-1", "1.5")])));
    assert_eq!(
        drained.daily_user.unwrap()["day-1"]["prompt_tokens"],
        SpendColumnValue::Int(1)
    );
    assert_eq!(drained.window.unwrap()[0].spend, amount("3.5"));
}
