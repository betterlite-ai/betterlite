#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::types::Config;

use super::{build, shared_settings};

const REDIS: &str = "router_settings:\n  redis_host: 127.0.0.1\n  redis_port: 6399\n";

fn config(source: &str) -> Config {
    ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(source)
    .unwrap()
}

fn config_with_env(source: &str, entries: &[(&str, &str)]) -> Config {
    let lookup: BTreeMap<String, String> = entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }.load(source).unwrap()
}

#[test]
fn a_config_without_redis_keeps_this_workers_counters_alone() {
    let service = build(&config("model_list: []\n")).unwrap();
    assert!(service.shared.is_none());
}

#[test]
fn a_config_with_redis_host_and_port_builds_the_shared_counters() {
    let service = build(&config(REDIS)).unwrap();
    assert!(service.shared.is_some());
}

#[test]
fn a_config_with_a_redis_url_builds_the_shared_counters() {
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
fn the_breaker_settings_come_from_the_redis_failure_policy_section() {
    let config = config_with_env(REDIS, &[("REDIS_CIRCUIT_BREAKER_FAILURE_THRESHOLD", "9")]);
    let settings = shared_settings(&config).unwrap();
    assert_eq!(settings.breaker.failure_threshold, 9);
    assert_eq!(settings.redis.host.as_deref(), Some("127.0.0.1"));
    assert_eq!(settings.redis.port, Some(6399));
}
