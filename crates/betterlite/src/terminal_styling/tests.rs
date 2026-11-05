#![cfg(test)]

use rstest::rstest;
use std::collections::BTreeMap;
use std::io::{self, IsTerminal as _};

use super::{blue, bold, color, dim, green, grey, orange, supports_color};

#[test]
fn no_color_present_disables_styling_regardless_of_terminal() {
    let environment = BTreeMap::from([(String::from("NO_COLOR"), String::new())]);
    assert!(!supports_color(&environment));
}

#[test]
fn absent_no_color_uses_terminal_capability() {
    let environment = BTreeMap::new();
    assert_eq!(supports_color(&environment), io::stdout().is_terminal());
}

#[rstest]
#[case::bold_enabled(true, "\x1b[1mhi\x1b[0m")]
#[case::bold_disabled(false, "hi")]
fn bold_wraps_only_when_supported(#[case] supports: bool, #[case] expected: &str) {
    assert_eq!(bold("hi", supports), expected);
}

#[rstest]
#[case::orange_enabled(true, "\x1b[38;2;215;119;87morange\x1b[0m")]
#[case::orange_disabled(false, "orange")]
fn orange_wraps_only_when_supported(#[case] supports: bool, #[case] expected: &str) {
    assert_eq!(orange("orange", supports), expected);
}

#[rstest]
#[case::grey_enabled(true, "\x1b[38;2;153;153;153mgrey\x1b[0m")]
#[case::grey_disabled(false, "grey")]
fn grey_wraps_only_when_supported(#[case] supports: bool, #[case] expected: &str) {
    assert_eq!(grey("grey", supports), expected);
}

#[rstest]
#[case::dim_enabled(true, "\x1b[2mdim\x1b[0m")]
#[case::dim_disabled(false, "dim")]
fn dim_wraps_only_when_supported(#[case] supports: bool, #[case] expected: &str) {
    assert_eq!(dim("dim", supports), expected);
}

#[rstest]
#[case::green_enabled(true, "\x1b[38;2;78;186;101mgreen\x1b[0m")]
#[case::green_disabled(false, "green")]
fn green_wraps_only_when_supported(#[case] supports: bool, #[case] expected: &str) {
    assert_eq!(green("green", supports), expected);
}

#[rstest]
#[case::blue_enabled(true, "\x1b[38;2;177;185;249mblue\x1b[0m")]
#[case::blue_disabled(false, "blue")]
fn blue_wraps_only_when_supported(#[case] supports: bool, #[case] expected: &str) {
    assert_eq!(blue("blue", supports), expected);
}

#[test]
fn color_applies_an_arbitrary_code_and_resets_it() {
    assert_eq!(
        color("\x1b[4m", "underline", true),
        "\x1b[4munderline\x1b[0m"
    );
    assert_eq!(color("\x1b[4m", "underline", false), "underline");
}
