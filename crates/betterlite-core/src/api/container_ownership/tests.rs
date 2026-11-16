#![cfg(test)]

mod access;
mod forwarding;
mod identity;
mod list;
mod postgres;
mod record;
mod response;
mod support;

use super::cache::{
    CachedText, ContainerCaches, NEGATIVE_OWNER_SENTINEL, NEGATIVE_STORED_ID_SENTINEL,
    allowed_ids_cache_key,
};
use support::{advance, fixed_clock, movable_clock};

#[test]
fn cache_owner_round_trips_an_owner() {
    let cache = ContainerCaches::new(fixed_clock()).unwrap();
    assert_eq!(
        cache.owner.read("container:openai:c1").unwrap(),
        CachedText::Miss
    );
    cache
        .owner
        .write("container:openai:c1", Some("alice"))
        .unwrap();
    assert_eq!(
        cache.owner.read("container:openai:c1").unwrap(),
        CachedText::Known("alice".to_owned())
    );
}

#[test]
fn cache_owner_stores_the_untracked_sentinel() {
    let cache = ContainerCaches::new(fixed_clock()).unwrap();
    cache.owner.write("container:openai:c1", None).unwrap();
    assert_eq!(
        cache.owner.read("container:openai:c1").unwrap(),
        CachedText::Absent
    );
}

#[test]
fn cache_owner_expires_after_the_ttl_boundary() {
    let (clock, now) = movable_clock();
    let cache = ContainerCaches::new(clock).unwrap();
    cache
        .owner
        .write("container:openai:c1", Some("alice"))
        .unwrap();
    advance(&now, 1_060.0);
    assert_eq!(
        cache.owner.read("container:openai:c1").unwrap(),
        CachedText::Known("alice".to_owned())
    );
    advance(&now, 1_061.0);
    assert_eq!(
        cache.owner.read("container:openai:c1").unwrap(),
        CachedText::Miss
    );
}

#[test]
fn cache_stored_id_maps_absent_and_empty_to_the_sentinel() {
    let cache = ContainerCaches::new(fixed_clock()).unwrap();
    assert_eq!(
        cache.stored_id.read("container:openai:c1").unwrap(),
        CachedText::Miss
    );
    cache.stored_id.write("container:openai:c1", None).unwrap();
    assert_eq!(
        cache.stored_id.read("container:openai:c1").unwrap(),
        CachedText::Absent
    );
    cache
        .stored_id
        .write("container:openai:c1", Some(""))
        .unwrap();
    assert_eq!(
        cache.stored_id.read("container:openai:c1").unwrap(),
        CachedText::Absent
    );
    cache
        .stored_id
        .write("container:openai:c1", Some("cntr_encoded"))
        .unwrap();
    assert_eq!(
        cache.stored_id.read("container:openai:c1").unwrap(),
        CachedText::Known("cntr_encoded".to_owned())
    );
}

#[test]
fn cache_keeps_a_json_literal_text_unchanged() {
    let cache = ContainerCaches::new(fixed_clock()).unwrap();
    cache
        .owner
        .write("container:openai:c1", Some("\"bob\""))
        .unwrap();
    assert_eq!(
        cache.owner.read("container:openai:c1").unwrap(),
        CachedText::Known("\"bob\"".to_owned())
    );
    cache
        .stored_id
        .write("container:openai:c1", Some("42"))
        .unwrap();
    assert_eq!(
        cache.stored_id.read("container:openai:c1").unwrap(),
        CachedText::Known("42".to_owned())
    );
}

#[test]
fn cache_owner_keeps_an_empty_owner_apart_from_the_untracked_answer() {
    let cache = ContainerCaches::new(fixed_clock()).unwrap();
    cache.owner.write("container:openai:c1", Some("")).unwrap();
    assert_eq!(
        cache.owner.read("container:openai:c1").unwrap(),
        CachedText::Known(String::new())
    );
}

#[test]
fn cache_sentinels_match_litellm() {
    assert_eq!(NEGATIVE_OWNER_SENTINEL, "__litellm_container_no_owner__");
    assert_eq!(
        NEGATIVE_STORED_ID_SENTINEL,
        "__litellm_container_no_stored_id__"
    );
}

#[test]
fn cache_allowed_round_trips_and_forgets() {
    let cache = ContainerCaches::new(fixed_clock()).unwrap();
    assert_eq!(cache.allowed("[\"a\"]").unwrap(), None);
    cache
        .store_allowed("[\"a\"]", &["container:openai:c1".to_owned()])
        .unwrap();
    assert_eq!(
        cache.allowed("[\"a\"]").unwrap(),
        Some(vec!["container:openai:c1".to_owned()])
    );
    cache.forget_allowed("[\"a\"]");
    assert_eq!(cache.allowed("[\"a\"]").unwrap(), None);
}

#[test]
fn cache_allowed_key_is_order_insensitive() {
    let first = allowed_ids_cache_key(&["team:t1".to_owned(), "alice".to_owned()]).unwrap();
    let second = allowed_ids_cache_key(&["alice".to_owned(), "team:t1".to_owned()]).unwrap();
    assert_eq!(first, second);
    assert_eq!(first, "[\"alice\",\"team:t1\"]");
}

#[test]
fn cache_allowed_key_keeps_separators_apart() {
    let joined = allowed_ids_cache_key(&["a|b".to_owned()]).unwrap();
    let split = allowed_ids_cache_key(&["a".to_owned(), "b".to_owned()]).unwrap();
    assert_ne!(joined, split);
    assert_eq!(joined, "[\"a|b\"]");
    assert_eq!(split, "[\"a\",\"b\"]");
    let quoted = allowed_ids_cache_key(&["a\"b".to_owned()]).unwrap();
    assert_eq!(quoted, "[\"a\\\"b\"]");
}
