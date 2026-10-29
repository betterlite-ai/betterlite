#![cfg(test)]

use rstest::rstest;

use super::check_and_fix_namespace;

#[rstest]
#[case::prepends_when_absent(Some("cache"), "key1", Some("cache:key1"))]
#[case::leaves_already_prefixed_unchanged(Some("cache"), "cache:key1", Some("cache:key1"))]
#[case::prefixes_bare_namespace_name(Some("cache"), "cache", Some("cache:cache"))]
#[case::none_namespace_leaves_key_unchanged(None, "key1", Some("key1"))]
#[case::empty_namespace_leaves_key_unchanged(Some(""), "key1", Some("key1"))]
fn prefixes_keys_idempotently(
    #[case] namespace: Option<&str>,
    #[case] key: &str,
    #[case] expected: Option<&str>,
) {
    assert_eq!(
        check_and_fix_namespace(namespace, Some(key)),
        expected.map(str::to_owned)
    );
}

#[rstest]
#[case::with_namespace(Some("cache"))]
#[case::without_namespace(None)]
fn preserves_none_key(#[case] namespace: Option<&str>) {
    assert_eq!(check_and_fix_namespace(namespace, None), None);
}

#[test]
fn reapplying_stays_idempotent() {
    let once = check_and_fix_namespace(Some("cache"), Some("key1"));
    let twice = check_and_fix_namespace(Some("cache"), once.as_deref());
    assert_eq!(once, twice);
}
