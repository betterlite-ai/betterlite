#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::routing_state_store::connection::Connection;
use betterlite_config::types::Config;
use betterlite_router::request_scheduler::flow::FlowItem;
use betterlite_router::routing_state_store::Settings as SharedSettings;
use betterlite_router::routing_state_store::test_support::FakeRedis;
use betterlite_router::routing_state_store::{RedisSettings, RoutingStateStore};
use betterlite_router::routing_value_store::{RouteKey, RouteValue, StoreRead};

use crate::routing::redis_failure_policy::policy_settings;

use super::build;

fn config(source: &str) -> Config {
    ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(source)
    .expect("the fixture config loads")
}

fn redis_config(port: u16) -> Config {
    config(&format!(
        "router_settings:\n  redis_host: 127.0.0.1\n  redis_port: {port}\n"
    ))
}

fn item(priority: u8, request_id: &str) -> FlowItem {
    FlowItem {
        priority,
        request_id: request_id.to_owned(),
        model: "gpt-4o".to_owned(),
    }
}

fn reader(port: u16) -> RoutingStateStore {
    let settings = SharedSettings {
        redis: RedisSettings {
            url: None,
            host: Some("127.0.0.1".to_owned()),
            port: Some(i64::from(port)),
            password: None,
            namespace: None,
            connection: Connection::default(),
            socket_timeout: 0.25,
            pool_timeout_seconds: 5,
            environment: BTreeMap::new(),
        },
        breaker: policy_settings(&config("model_list: []\n").sections.redis_failure_policy),
    };
    RoutingStateStore::new(&settings, Arc::new(|| Duration::ZERO)).expect("the reader builds")
}

#[tokio::test]
async fn a_config_without_redis_builds_a_local_queue() {
    let scheduler = build(&config("model_list: []\n")).expect("the scheduler builds");
    scheduler
        .add_request(&item(0, "local"))
        .await
        .expect("the request queues");
    let queued = scheduler.queue("gpt-4o").await.expect("the queue reads");
    assert_eq!(queued.warning, None);
    assert_eq!(queued.value.len(), 1);
}

// the config's redis host and port reach the queue: an independent reader sees the entry the scheduler wrote
#[tokio::test]
async fn a_config_with_redis_writes_the_queue_the_shared_tier_holds() {
    let server = FakeRedis::try_start().await.expect("the fake redis starts");
    let scheduler = build(&redis_config(server.port())).expect("the scheduler builds");
    scheduler
        .add_request(&item(128, "shared"))
        .await
        .expect("the request queues");
    let read = reader(server.port())
        .get(&RouteKey::parse("scheduler:queue:gpt-4o").expect("the key"))
        .await
        .expect("the shared entry reads");
    assert_eq!(
        read.value,
        StoreRead::Value(RouteValue::Members(BTreeSet::from([
            "128:shared".to_owned()
        ])))
    );
}

// str(uuid.uuid4()) is the hyphenated 36-character form
#[test]
fn request_ids_are_uuids() {
    let generated = super::request_ids()();
    assert_eq!(generated.len(), 36);
    assert_eq!(generated.matches('-').count(), 4);
}
