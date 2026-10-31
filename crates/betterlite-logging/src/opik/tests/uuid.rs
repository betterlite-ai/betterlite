#![cfg(test)]

use betterlite_logging::opik::uuid7::create_uuid7;

#[test]
fn uuid_is_v7_with_rfc_variant() {
    let uuid = uuid::Uuid::parse_str(&create_uuid7()).unwrap();
    assert_eq!(uuid.get_version_num(), 7);
    assert_eq!(uuid.get_variant(), uuid::Variant::RFC4122);
}
