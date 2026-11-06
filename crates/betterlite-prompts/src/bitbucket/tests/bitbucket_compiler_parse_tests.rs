use betterlite_prompts::bitbucket::messages::parse_prompt_to_messages;

use crate::support::messages;

#[test]
fn no_role_prefix_single_user_message() {
    let parsed = parse_prompt_to_messages("  first line\n\n  second line  ");
    assert_eq!(
        parsed,
        messages(&[("user", "first line\n\nsecond line")]),
        "an unprefixed template keeps its interior blank line"
    );
}

#[test]
fn blank_template_empty_list() {
    assert!(parse_prompt_to_messages("   \n\t\n \n").is_empty());
    assert!(parse_prompt_to_messages("").is_empty());
}

#[test]
fn roles_are_matched_case_insensitively_and_written_lowercase() {
    let parsed = parse_prompt_to_messages("SYSTEM: rules\nUser: hello\nAssistant: hi");
    assert_eq!(
        parsed,
        messages(&[("system", "rules"), ("user", "hello"), ("assistant", "hi"),])
    );
}

#[test]
fn blank_lines_are_skipped_and_do_not_end_a_message() {
    let parsed = parse_prompt_to_messages("User: one\n\n  two  \n\n\nUser: three");
    assert_eq!(parsed, messages(&[("user", "one\ntwo"), ("user", "three")]));
}

#[test]
fn content_after_a_prefix_is_trimmed_and_keeps_inner_colons() {
    let parsed = parse_prompt_to_messages("User:   see http://example.test:80  ");
    assert_eq!(parsed, messages(&[("user", "see http://example.test:80")]));
}

#[test]
fn a_role_prefix_without_content_makes_an_empty_message() {
    let parsed = parse_prompt_to_messages("System:\nUser: hi");
    assert_eq!(parsed, messages(&[("system", ""), ("user", "hi")]));
}

#[test]
fn unprefixed_lines_before_the_first_role_are_dropped() {
    let parsed = parse_prompt_to_messages("preamble\nUser: hello");
    assert_eq!(parsed, messages(&[("user", "hello")]));
}
