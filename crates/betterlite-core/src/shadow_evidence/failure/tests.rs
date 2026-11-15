#![cfg(test)]

use rstest::rstest;

use super::{Location, detail};

#[test]
fn detail_names_the_class_and_the_call_site() {
    let location = Location {
        file: "/srv/app/litellm/router.py",
        line: 42,
    };
    assert_eq!(
        detail("ValueError", Some(location), "boom"),
        "ValueError at router.py:42: boom"
    );
}

#[rstest]
#[case::nested_path("/srv/app/deep/mod.py", "mod.py")]
#[case::top_level("mod.py", "mod.py")]
#[case::windows_path(r"C:\app\mod.py", r"C:\app\mod.py")]
#[case::trailing_slash("/srv/app/", "")]
#[case::empty_path("", "")]
fn basename_keeps_the_last_slash_segment(#[case] file: &str, #[case] expected: &str) {
    let location = Location { file, line: 7 };
    assert_eq!(
        detail("TimeoutError", Some(location), "lost"),
        format!("TimeoutError at {expected}:7: lost")
    );
}

#[test]
fn detail_without_a_call_site_keeps_the_class() {
    assert_eq!(
        detail("RuntimeError", None, "no traceback"),
        "RuntimeError: no traceback"
    );
}

#[test]
fn detail_displays_the_message_value() {
    assert_eq!(detail("KeyError", None, 5_i32), "KeyError: 5");
}

#[rstest]
#[case::just_inside_the_cap(496, 496)]
#[case::exactly_the_cap(497, 497)]
#[case::one_past_the_cap(498, 497)]
#[case::far_past_the_cap(600, 497)]
fn detail_stops_at_five_hundred_characters(
    #[case] message_chars: usize,
    #[case] kept_chars: usize,
) {
    let message = "a".repeat(message_chars);
    assert_eq!(
        detail("E", None, &message),
        format!("E: {}", "a".repeat(kept_chars))
    );
}

#[test]
fn detail_counts_characters_not_bytes() {
    let message = "\u{e9}".repeat(600);
    let detail = detail("E", None, message);
    assert_eq!(detail.chars().count(), 500);
    assert_eq!(detail, format!("E: {}", "\u{e9}".repeat(497)));
}

#[test]
fn detail_cap_applies_after_the_location() {
    let location = Location {
        file: "mod.py",
        line: 1,
    };
    let message = "\u{e9}".repeat(600);
    let detail = detail("E", Some(location), message);
    assert_eq!(detail.chars().count(), 500);
    assert_eq!(detail, format!("E at mod.py:1: {}", "\u{e9}".repeat(485)));
}
