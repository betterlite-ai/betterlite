use std::sync::Arc;
use std::time::Duration;

use rmcp::model::{NumberOrString, ProgressNotificationParam, ProgressToken};
use tokio::sync::mpsc::Receiver;
use tokio::time::timeout;

use crate::client::sdk_runtime::handler::progress::ProgressRegistry;

fn token(name: &str) -> ProgressToken {
    ProgressToken(NumberOrString::String(Arc::from(name)))
}

fn update(token: &ProgressToken, step: f64) -> ProgressNotificationParam {
    ProgressNotificationParam::new(token.clone(), step)
}

async fn next(
    receiver: &mut Receiver<ProgressNotificationParam>,
) -> Option<ProgressNotificationParam> {
    timeout(Duration::from_secs(5), receiver.recv())
        .await
        .expect("the progress update arrived")
}

fn fill(registry: &ProgressRegistry, token: &ProgressToken, count: u32) {
    for step in 1..=count {
        registry.deliver(update(token, f64::from(step)));
    }
}

#[tokio::test]
async fn updates_that_arrive_before_their_subscription_are_kept_per_token() {
    let registry = Arc::new(ProgressRegistry::default());
    let first = token("first");
    let second = token("second");
    registry.deliver(update(&first, 1.0));
    registry.deliver(update(&second, 2.0));
    let (mut updates, _subscription) = registry.subscribe(&first);
    let received = next(&mut updates).await.expect("the first token's update");
    assert_eq!(received.progress.to_bits(), 1.0_f64.to_bits());
    assert!(
        updates.try_recv().is_err(),
        "the second token's update stays queued for its own subscriber"
    );
    let (mut later, _later_subscription) = registry.subscribe(&second);
    let second_update = next(&mut later).await.expect("the second token's update");
    assert_eq!(second_update.progress.to_bits(), 2.0_f64.to_bits());
}

#[tokio::test]
async fn a_dropped_subscription_hands_its_updates_back() {
    let registry = Arc::new(ProgressRegistry::default());
    let token = token("returned");
    let (mut stale, subscription) = registry.subscribe(&token);
    drop(subscription);
    registry.deliver(update(&token, 7.0));
    let (mut fresh, _fresh_subscription) = registry.subscribe(&token);
    let received = next(&mut fresh).await.expect("the returned update");
    assert_eq!(received.progress.to_bits(), 7.0_f64.to_bits());
    assert!(
        stale.try_recv().is_err(),
        "a dropped subscription receives nothing"
    );
}

#[tokio::test]
async fn the_early_queue_drops_its_oldest_update_at_its_limit() {
    // the registry keeps 64 updates that arrive before a subscription
    let registry = Arc::new(ProgressRegistry::default());
    let oldest = token("oldest");
    let filler = token("filler");
    registry.deliver(update(&oldest, 0.0));
    fill(&registry, &filler, 63);
    registry.deliver(update(&oldest, 1.0));
    let (mut updates, _subscription) = registry.subscribe(&oldest);
    let received = next(&mut updates).await.expect("the newest update");
    assert_eq!(
        received.progress.to_bits(),
        1.0_f64.to_bits(),
        "the oldest update fell out of the queue"
    );
    assert!(
        updates.try_recv().is_err(),
        "only the newest update for the token remains"
    );
}
