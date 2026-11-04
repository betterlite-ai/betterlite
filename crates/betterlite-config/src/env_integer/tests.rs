#![cfg(test)]

use super::*;

#[test]
fn integer_environment_fallbacks_are_explicit() {
    assert_eq!(get_env_int("COUNT", Some(" 12 "), 5), 12);
    assert_eq!(get_env_int("COUNT", Some("bad"), 5), 5);
    assert_eq!(get_env_int_in_range("COUNT", Some("99"), 5, 0, 10), 5);
    assert_eq!(get_env_int_or_none(Some("")), None);
    assert_eq!(get_env_int_or_none(Some("7")), Some(7));
}
