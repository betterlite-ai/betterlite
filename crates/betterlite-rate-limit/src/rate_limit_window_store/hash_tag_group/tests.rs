#![cfg(test)]

use super::{REDIS_NODE_HASHTAG_NAME, group_keys_by_hash_tag};
use crate::rate_limit_window_store::RedisCacheKind;
use crate::rate_limit_window_store::slot_compute::keyslot_for_redis_cluster;
use crate::rate_limit_window_store::tests::support::{FakeCache, key_list, store_with};

#[test]
fn a_standalone_cache_groups_every_key_under_all_keys() {
    let store = store_with(FakeCache::new(Some(RedisCacheKind::Standalone)));
    let keys = key_list(&["foo", "bar", "{tag}baz"]);
    assert_eq!(
        group_keys_by_hash_tag(&store, &keys),
        vec![(REDIS_NODE_HASHTAG_NAME.to_owned(), keys)]
    );
}

#[test]
fn a_cache_without_redis_groups_every_key_under_all_keys() {
    let store = store_with(FakeCache::new(None));
    let keys = key_list(&["foo", "bar"]);
    assert_eq!(
        group_keys_by_hash_tag(&store, &keys),
        vec![(REDIS_NODE_HASHTAG_NAME.to_owned(), keys)]
    );
}

#[test]
fn a_cluster_groups_every_key_under_its_slot() {
    let store = store_with(FakeCache::new(Some(RedisCacheKind::Cluster)));
    let groups = group_keys_by_hash_tag(&store, &key_list(&["foo", "bar"]));
    assert_eq!(groups.len(), 2);
    assert_eq!(
        groups[0],
        (
            format!("slot_{}", keyslot_for_redis_cluster("foo")),
            key_list(&["foo"])
        )
    );
    assert_eq!(
        groups[1],
        (
            format!("slot_{}", keyslot_for_redis_cluster("bar")),
            key_list(&["bar"])
        )
    );
}

#[test]
fn keys_with_the_same_slot_share_a_group_in_first_seen_order() {
    let store = store_with(FakeCache::new(Some(RedisCacheKind::Cluster)));
    let groups = group_keys_by_hash_tag(
        &store,
        &key_list(&["{user1000}.following", "foo", "{user1000}.followers"]),
    );
    assert_eq!(groups.len(), 2);
    assert_eq!(
        groups[0].1,
        key_list(&["{user1000}.following", "{user1000}.followers"])
    );
    assert_eq!(groups[1].1, key_list(&["foo"]));
}

#[test]
fn every_group_tag_is_the_slot_number() {
    let store = store_with(FakeCache::new(Some(RedisCacheKind::Cluster)));
    let groups = group_keys_by_hash_tag(&store, &key_list(&["foo"]));
    assert_eq!(groups[0].0, format!("slot_{}", 0x2F96_u16));
}
