#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::types::Config;
use betterlite_router::affinity_claims::identity::AffinityKey;
use betterlite_router::affinity_claims::pin::{AffinityPin, StoredPin};
use indexmap::IndexMap;

use super::{build, shared_settings};

const REDIS: &str = "router_settings:\n  redis_host: 127.0.0.1\n  redis_port: 6399\n";

fn config(source: &str) -> Config {
    ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(source)
    .unwrap()
}

#[test]
fn a_config_without_redis_keeps_this_workers_pins_alone() {
    let service = build(&config("model_list: []\n")).unwrap();
    assert!(service.shared.is_none());
}

#[test]
fn a_config_with_redis_host_and_port_builds_the_shared_pins() {
    let service = build(&config(REDIS)).unwrap();
    assert!(service.shared.is_some());
}

#[test]
fn a_config_with_a_redis_url_builds_the_shared_pins() {
    let service = build(&config(
        "router_settings:\n  redis_url: redis://127.0.0.1:6399\n",
    ))
    .unwrap();
    assert!(service.shared.is_some());
}

#[test]
fn a_config_without_redis_names_no_shared_settings() {
    assert!(shared_settings(&config("model_list: []\n")).is_none());
}

#[test]
fn the_local_pin_cache_takes_a_claim() {
    let service = build(&config("model_list: []\n")).unwrap();
    let key = AffinityKey::parse("deployment_affinity:v1:group:caller").unwrap();
    let pin = AffinityPin(IndexMap::from([(
        "model_id".to_owned(),
        "deployment-1".to_owned(),
    )]));
    let winner = service.local.claim_in_memory(&key, &pin, 60, None).unwrap();
    assert_eq!(winner, StoredPin::Pin(pin));
}
