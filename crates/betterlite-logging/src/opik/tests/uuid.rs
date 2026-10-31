#![cfg(test)]

use betterlite_logging::opik::uuid7::create_uuid7;
use chrono::Utc;

#[test]
fn uuid_is_v7_with_rfc_variant() {
    let uuid = uuid::Uuid::parse_str(&create_uuid7()).unwrap();
    assert_eq!(uuid.get_version_num(), 7);
    assert_eq!(uuid.get_variant(), uuid::Variant::RFC4122);
}

#[test]
fn uuid_embeds_the_millisecond_timestamp_in_the_first_48_bits() {
    let now_ms = Utc::now().timestamp_millis();
    let uuid = uuid::Uuid::parse_str(&create_uuid7()).unwrap();
    let bytes = uuid.into_bytes();
    let mut timestamp_bytes = [0u8; 8];
    timestamp_bytes[2..].copy_from_slice(&bytes[..6]);
    let embedded_ms = i64::from_be_bytes(timestamp_bytes);
    assert!((embedded_ms - now_ms).abs() < 5000);
}
