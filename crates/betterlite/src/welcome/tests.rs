#![cfg(test)]

use super::{DIVIDER_CHAR, LITELLM_ASCII, print_welcome};

#[test]
fn prints_the_banner_version_line_and_divider() {
    let mut writer = Vec::new();

    print_welcome("1.2.3", false, &mut writer).expect("writing succeeds");

    let output = String::from_utf8(writer).expect("writer output is valid utf8");
    assert_eq!(
        output,
        format!(
            "\n{LITELLM_ASCII}\n  Welcome to LiteLLM v1.2.3\n\n  {}\n\n",
            DIVIDER_CHAR.to_string().repeat(74)
        )
    );
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
    assert!(output.contains(
        "  \x1b[38;2;215;119;87mWelcome\x1b[0m to \x1b[1mLiteLLM\x1b[0m \x1b[38;2;153;153;153mv1.2.3\x1b[0m"
    ));
}
