#![cfg(test)]

#[test]
fn uuid_is_v7_with_rfc_variant() {
    let uuid = uuid::Uuid::parse_str(&betterlite_logging::opik::create_uuid7()).unwrap();
    assert_eq!(uuid.get_version_num(), 7);
    assert_eq!(uuid.get_variant(), uuid::Variant::RFC4122);
}
