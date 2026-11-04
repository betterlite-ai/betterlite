#![cfg(test)]

use super::print_welcome;

#[test]
fn prints_the_banner_version_line_and_divider() {
    let mut writer = Vec::new();

    print_welcome("1.2.3", false, &mut writer).expect("writing succeeds");

    let output = String::from_utf8(writer).expect("writer output is valid utf8");
    assert!(output.contains(&"\u{2588}".repeat(8)));
    assert!(output.contains("Welcome to LiteLLM v1.2.3"));
    assert!(output.contains(&"\u{254c}".repeat(74)));
}

#[test]
fn applies_no_ansi_codes_when_color_is_unsupported() {
    let mut writer = Vec::new();

    print_welcome("1.2.3", false, &mut writer).expect("writing succeeds");

    let output = String::from_utf8(writer).expect("writer output is valid utf8");
    assert!(!output.contains('\u{1b}'));
}

#[test]
fn wraps_the_welcome_line_in_ansi_codes_when_color_is_supported() {
    let mut writer = Vec::new();

    print_welcome("1.2.3", true, &mut writer).expect("writing succeeds");

    let output = String::from_utf8(writer).expect("writer output is valid utf8");
    assert!(output.contains('\u{1b}'));
    assert!(output.contains("Welcome"));
    assert!(output.contains("LiteLLM"));
    assert!(output.contains("v1.2.3"));
}
