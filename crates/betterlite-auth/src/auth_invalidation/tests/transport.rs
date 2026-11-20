use std::sync::Arc;

use super::support::{self, RecordingEndpoint, RecordingEviction};
use crate::auth_invalidation::transport::{InvalidationTransport, PubSubTransport};
use crate::auth_pubsub::publish::PublishTasks;

fn transport(endpoint: &Arc<RecordingEndpoint>) -> PubSubTransport<RecordingEndpoint> {
    PubSubTransport {
        tasks: PublishTasks::default(),
        endpoint: Some(Arc::clone(endpoint)),
    }
}

#[test]
fn publish_encodes_and_sends_one_invalidation() {
    let runtime = support::runtime();
    let endpoint = Arc::new(RecordingEndpoint::default());
    let transport = transport(&endpoint);
    runtime.block_on(async {
        InvalidationTransport::publish(&transport, "team_id:team-1", Some(4.0_f64), Some(60.0_f64))
            .await;
        support::settle().await;
    });
    assert_eq!(
        endpoint.published(),
        vec![r#"{"cache_key":"team_id:team-1","new_value":4,"ttl":60}"#.to_owned()]
    );
}

#[test]
fn evict_and_broadcast_applies_locally_then_publishes_each_key() {
    let runtime = support::runtime();
    let endpoint = Arc::new(RecordingEndpoint::default());
    let transport = transport(&endpoint);
    let eviction = RecordingEviction::default();
    let keys = vec!["team_a".to_owned(), "team_b".to_owned()];
    runtime.block_on(async {
        InvalidationTransport::evict_and_broadcast(&transport, &keys, &eviction).await;
        support::settle().await;
    });
    assert_eq!(eviction.applied(), keys);
    let mut published = endpoint.published();
    published.sort();
    assert_eq!(
        published,
        vec![
            r#"{"cache_key":"team_a"}"#.to_owned(),
            r#"{"cache_key":"team_b"}"#.to_owned(),
        ]
    );
}
