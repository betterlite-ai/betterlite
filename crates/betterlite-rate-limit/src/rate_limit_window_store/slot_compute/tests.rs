#![cfg(test)]

use rstest::rstest;

use super::{REDIS_CLUSTER_SLOTS, SLOT_BITS, hash_tag, keyslot_for_redis_cluster};

#[test]
fn the_crc16_check_value_matches_the_standard_vector() {
    // CRC-16/XMODEM("123456789") is 0x31C3, the value binascii.crc_hqx gives, and it is already below 16384
    assert_eq!(keyslot_for_redis_cluster("123456789"), 0x31C3);
}

#[test]
fn the_slot_count_and_its_mask_are_the_cluster_values() {
    assert_eq!(REDIS_CLUSTER_SLOTS, 0x4000);
    assert_eq!(SLOT_BITS, 0x3FFF);
}

#[rstest]
#[case::foo("foo", 0x2F96)]
#[case::bar("bar", 0x13C5)]
#[case::hello("hello", 866)]
#[case::tagged("a{tag}b", 0x2092)]
fn a_key_lands_on_the_slot_binascii_crc_hqx_gives(#[case] key: &str, #[case] slot: u16) {
    assert_eq!(keyslot_for_redis_cluster(key), slot);
    assert!(keyslot_for_redis_cluster(key) < REDIS_CLUSTER_SLOTS);
}

#[rstest]
#[case::plain("user1000", "user1000")]
#[case::tagged("{user1000}.following", "user1000")]
#[case::trailing_tag("a{tag}b", "tag")]
#[case::empty_tag("{}foo", "{}foo")]
#[case::no_closing_brace("a{tag", "a{tag")]
#[case::brace_after_before("x}y{tag}", "tag")]
fn a_key_hashes_its_hash_tag_or_the_whole_key(#[case] key: &str, #[case] hashed: &str) {
    assert_eq!(hash_tag(key), hashed);
}

#[test]
fn keys_with_the_same_tag_share_a_slot() {
    assert_eq!(
        keyslot_for_redis_cluster("{user1000}.following"),
        keyslot_for_redis_cluster("user1000")
    );
    assert_eq!(
        keyslot_for_redis_cluster("{user1000}.followers"),
        keyslot_for_redis_cluster("user1000")
    );
}

#[test]
fn an_empty_tag_hashes_the_whole_key() {
    // binascii.crc_hqx("{}foo", 0) % 16384 is 9500, and the tag alone would land on the slot of "foo"
    assert_eq!(keyslot_for_redis_cluster("{}foo"), 9500);
    assert_ne!(
        keyslot_for_redis_cluster("{}foo"),
        keyslot_for_redis_cluster("foo")
    );
}

#[test]
fn a_whitespace_tag_is_still_a_tag() {
    // Redis and LiteLLM use any non-empty text between the braces, whitespace included
    assert_eq!(hash_tag("{ }"), " ");
    assert_eq!(
        keyslot_for_redis_cluster("{ }"),
        keyslot_for_redis_cluster(" ")
    );
}
