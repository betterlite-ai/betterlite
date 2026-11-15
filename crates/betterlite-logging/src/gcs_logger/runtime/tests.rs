#![cfg(test)]

use super::{IdSource as _, RandomIds};

#[test]
fn a_random_id_is_thirty_two_lowercase_hex_digits() {
    let id = RandomIds.uuid_hex();
    assert_eq!(id.len(), 32);
    assert!(id.chars().all(|character| character.is_ascii_hexdigit()));
    assert!(id.chars().all(|character| !character.is_ascii_uppercase()));
}

#[test]
fn two_random_ids_differ() {
    let id = RandomIds.uuid_hex();
    assert_ne!(id, RandomIds.uuid_hex());
}
