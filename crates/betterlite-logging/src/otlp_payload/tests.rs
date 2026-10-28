#![cfg(test)]

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;

use super::{newest_that_fit, shrunk_message, truncate_payload};

// json.dumps writes the ellipsis as this escape inside a message
const ELLIPSIS: &str = concat!("\\", "u2026");

fn message(role: &str, content: &str) -> String {
    format!(r#"{{"role": "{role}", "content": "{content}"}}"#)
}

fn list(items: &[String]) -> String {
    format!("[{}]", items.join(", "))
}

fn cut(kept: &str, dropped: usize) -> String {
    format!("{kept}{ELLIPSIS}[truncated {dropped} bytes]")
}

fn elided(count: usize) -> String {
    message(
        "system",
        &format!("{ELLIPSIS}[{count} earlier messages truncated]"),
    )
}

#[rstest]
#[case::short("plain".to_owned(), "plain".to_owned())]
#[case::exactly_the_limit("x".repeat(0x1_0000), "x".repeat(0x1_0000))]
#[case::short_list(
    list(&[message("user", "a"), message("user", "b")]),
    list(&[message("user", "a"), message("user", "b")])
)]
#[case::not_a_list(
    "a".repeat(0x1_0001),
    format!("{}\u{2026}[truncated 1 bytes]", "a".repeat(0x1_0000))
)]
#[case::not_json(
    format!("[{}", "a".repeat(0x1_0000)),
    format!("[{}\u{2026}[truncated 1 bytes]", "a".repeat(0xffff))
)]
#[case::not_messages(
    format!("[{}1]", "1, ".repeat(29_999)),
    format!("[{}\u{2026}[truncated 24464 bytes]", "1, ".repeat(21_845))
)]
#[case::single_message(
    list(&[message("user", &"x".repeat(0x1_0000))]),
    format!(r#"[{{"role": "user", "content": "{}{}"#, "x".repeat(65_506), "\u{2026}[truncated 33 bytes]")
)]
#[case::exact_list_is_unchanged(
    list(&[message("user", &"a".repeat(65_435)), message("user", "b"), message("user", "c")]),
    list(&[message("user", &"a".repeat(65_435)), message("user", "b"), message("user", "c")])
)]
#[case::keeps_first_and_newest(
    list(&[[message("system", "s")].as_slice(), &vec![message("user", &"u".repeat(5000)); 20]].concat()),
    list(&[[message("system", "s"), elided(7)].as_slice(), &vec![message("user", &"u".repeat(5000)); 13]].concat())
)]
#[case::rest_fits_after_reencoding(
    format!("[{},{}{}, {}]", message("user", "a"), " ".repeat(70_000), message("user", "b"), message("user", "c")),
    list(&[message("user", "a"), elided(1), message("user", "c")])
)]
#[case::two_messages_are_shortened(
    format!("{}{}", list(&[message("user", "a"), message("user", "b")]), " ".repeat(70_000)),
    list(&[message("user", &cut("a", 0)), message("user", &cut("b", 0))])
)]
#[case::shortened_to_the_limit(
    list(&[message(&"r".repeat(32_710), &"c".repeat(10_000)), message(&"r".repeat(32_710), &"c".repeat(10_000))]),
    list(&[message(&"r".repeat(32_710), &cut("", 10_000)), message(&"r".repeat(32_710), &cut("", 10_000))])
)]
#[case::shortened_past_the_limit(
    list(&[message(&"r".repeat(32_711), &"c".repeat(10_000)), message(&"r".repeat(32_710), &"c".repeat(10_000))]),
    list(&[elided(2)])
)]
#[case::three_messages_are_shortened(
    list(&[message("user", &"a".repeat(70_000)), message("user", "b"), message("assistant", &"c".repeat(70_000))]),
    list(&[message("user", &cut(&"a".repeat(32_652), 37_348)), elided(1), message("assistant", &cut(&"c".repeat(32_647), 37_353))])
)]
#[case::other_fields_stay(
    list(&[r#"{"role": "user", "content": "PLACEHOLDER", "name": "n"}"#.replace("PLACEHOLDER", &"a".repeat(70_000)), message("assistant", &"c".repeat(70_000))]),
    list(&[r#"{"role": "user", "content": "PLACEHOLDER", "name": "n"}"#.replace("PLACEHOLDER", &cut(&"a".repeat(32_639), 37_361)), message("assistant", &cut(&"c".repeat(32_647), 37_353))])
)]
#[case::large_fields_are_dropped(
    list(&[format!(r#"{{"content": "a", "meta": "{}"}}"#, "m".repeat(70_000)), message("user", "z")]),
    list(&[message("user", &cut("a", 0)), message("user", &cut("z", 0))])
)]
#[case::missing_content_is_null(
    list(&[format!(r#"{{"content": "{}"}}"#, "a".repeat(70_000)), r#"{"text": "b"}"#.to_owned()]),
    list(&[format!(r#"{{"content": "{}"}}"#, cut(&"a".repeat(32_668), 37_332)), format!(r#"{{"text": "b", "content": "{}"}}"#, cut("null", 0))])
)]
#[case::escapes_are_budgeted(
    list(&[message("user", &"\u{e9}".repeat(40_000)), message("assistant", "c")]),
    list(&[message("user", &cut(&concat!("\\", "u00e9").repeat(2721), 74_558)), message("assistant", &cut("c", 0))])
)]
fn truncates_like_litellm(#[case] payload: String, #[case] expected: String) {
    assert_eq!(truncate_payload(&payload), expected);
}

#[rstest]
#[case::none_fit(1, 0)]
#[case::exactly_one_fits(2, 1)]
#[case::exactly_all_fit(5, 2)]
fn counts_the_newest_messages_that_fit(#[case] budget: usize, #[case] expected: usize) {
    let encoded = ["ab".to_owned(), "c".to_owned()];
    assert_eq!(newest_that_fit(&encoded, budget), expected);
}

#[test]
fn keeps_an_attempt_exactly_at_its_budget() {
    let content = format!("{}{}", "\"".repeat(21), "a".repeat(200));
    let original = IndexMap::from([
        ("role".to_owned(), Json::Text("user".to_owned())),
        ("content".to_owned(), Json::Text(content)),
    ]);
    let kept = format!("{}{}", "\\\"".repeat(21), "a".repeat(100));
    assert_eq!(
        shrunk_message(&original, 200),
        message("user", &cut(&kept, 100))
    );
}
