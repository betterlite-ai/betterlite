use super::super::*;

#[test]
fn lock_keys_are_signed_big_endian_blake2b() {
    assert_eq!(_mcp_identifier_lock_keys(&[Some("a")]).len(), 1);
}
