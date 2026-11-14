use rstest::rstest;

use crate::cache_prediction_targets::scope::cache_scope;

#[test]
fn scope_is_the_litellm_digest() {
    assert_eq!(
        cache_scope("caller", "deployment", "sk-ant-oat01-key", "claude-x"),
        "082a1b42cd7035077e977acd770ae6418cf067f36da9689acb55797a4c814fc5"
    );
}

#[test]
fn scope_hashes_non_ascii_text_as_utf8() {
    assert_eq!(
        cache_scope("h\u{e9}llo", "\u{4e16}\u{754c}", "", "x\"y\\z"),
        "3447059a94069de6c20dd8b111fa4152b51858cdea5333178ab3adce3aa1fc1f"
    );
}

#[test]
fn scope_hashes_the_empty_scope() {
    assert_eq!(
        cache_scope("", "", "", ""),
        "e9e6170f805c8ac5467b62857d27407a0f9f9db7d5c399001660e6ddd29b637a"
    );
}

#[rstest]
#[case::caller("other", "deployment", "key", "claude-x")]
#[case::deployment("caller", "other", "key", "claude-x")]
#[case::provider_key("caller", "deployment", "other", "claude-x")]
#[case::model("caller", "deployment", "key", "other")]
fn scope_hashes_each_field(
    #[case] caller: &str,
    #[case] deployment: &str,
    #[case] key: &str,
    #[case] model: &str,
) {
    assert_ne!(
        cache_scope(caller, deployment, key, model),
        cache_scope("caller", "deployment", "key", "claude-x")
    );
}

#[test]
fn scope_is_a_lowercase_hex_digest() {
    let scope = cache_scope("caller", "deployment", "key", "claude-x");
    assert_eq!(scope.len(), 64);
    assert!(
        scope
            .chars()
            .all(|found| found.is_ascii_hexdigit() && !found.is_ascii_uppercase())
    );
}
