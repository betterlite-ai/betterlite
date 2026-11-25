#![cfg(test)]

use std::collections::BTreeMap;

use super::settings::{
    DEFAULT_SETTINGS, HEALTH_CHECK_TTL_VARIABLE, LOCK_TTL_VARIABLE, SharedHealthSettings,
};
use super::support::{CHECKER, SHARED_AT, World, instant};

#[test]
fn the_defaults_are_the_sources_300_and_60() {
    let environment: BTreeMap<String, String> = BTreeMap::new();
    let settings = SharedHealthSettings::resolve(&environment).expect("the defaults resolve");
    assert_eq!(settings, DEFAULT_SETTINGS);
    assert_eq!(settings.health_check_ttl, 300);
    assert_eq!(settings.lock_ttl, 60);
}

#[test]
fn the_variable_names_are_the_litellm_ones() {
    assert_eq!(HEALTH_CHECK_TTL_VARIABLE, "DEFAULT_SHARED_HEALTH_CHECK_TTL");
    assert_eq!(LOCK_TTL_VARIABLE, "DEFAULT_SHARED_HEALTH_CHECK_LOCK_TTL");
}

#[test]
fn each_ttl_reads_its_own_variable() {
    let mut environment = BTreeMap::new();
    environment.insert(HEALTH_CHECK_TTL_VARIABLE.to_owned(), "90".to_owned());
    environment.insert(LOCK_TTL_VARIABLE.to_owned(), "15".to_owned());
    let settings = SharedHealthSettings::resolve(&environment).expect("the overrides resolve");
    assert_eq!(settings.health_check_ttl, 90);
    assert_eq!(settings.lock_ttl, 15);
}

#[test]
fn an_empty_variable_reads_as_unset() {
    let mut environment = BTreeMap::new();
    environment.insert(HEALTH_CHECK_TTL_VARIABLE.to_owned(), String::new());
    environment.insert(LOCK_TTL_VARIABLE.to_owned(), String::new());
    let settings = SharedHealthSettings::resolve(&environment).expect("empty values are unset");
    assert_eq!(settings, DEFAULT_SETTINGS);
}

#[test]
fn an_unparsable_ttl_names_its_variable() {
    let mut environment = BTreeMap::new();
    environment.insert(HEALTH_CHECK_TTL_VARIABLE.to_owned(), "soon".to_owned());
    let error = SharedHealthSettings::resolve(&environment).expect_err("a word is not a TTL");
    assert!(
        error
            .to_string()
            .contains("DEFAULT_SHARED_HEALTH_CHECK_TTL")
    );
}

#[test]
fn the_manager_takes_its_ttls_and_pod_id_from_the_settings() {
    let world = World::new(instant(SHARED_AT));
    let settings = SharedHealthSettings {
        health_check_ttl: 45,
        lock_ttl: 12,
    };
    let manager = world.manager_with(settings);
    assert_eq!(manager.health_check_ttl, 45);
    assert_eq!(manager.lock_ttl, 12);
    assert_eq!(manager.pod_id, CHECKER);
    assert!(manager.redis.is_some());
}

#[test]
fn a_manager_without_redis_keeps_the_defaults() {
    let world = World::new(instant(SHARED_AT));
    let manager = world.local_manager();
    assert!(manager.redis.is_none());
    assert_eq!(manager.health_check_ttl, 300);
    assert_eq!(manager.lock_ttl, 60);
}
