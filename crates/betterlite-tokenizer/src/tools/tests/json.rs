use rstest::rstest;

use crate::tools::json::{Scanner, kind_name, max_depth};

#[rstest]
#[case::nested_arrays(b"[[[]]]", 3)]
#[case::braces_inside_strings(br#"["a{"]"#, 1)]
#[case::string_then_nested_array(br#"{"a":[1]}"#, 2)]
#[case::escaped_quote_then_containers(b"[\"a\\\"]{[\"]", 1)]
#[case::closed_before_deeper_arrays(b"[[],[[]]]", 3)]
fn token_formatting_behavior_tests_behavior(#[case] bytes: &[u8], #[case] expected: usize) {
    assert_eq!(max_depth(bytes), expected);
}

#[rstest]
#[case(b'{', "an object")]
#[case(b'[', "an array")]
#[case(b'"', "a string")]
#[case(b't', "a boolean")]
#[case(b'f', "a boolean")]
#[case(b'n', "null")]
#[case(b'9', "a number")]
fn names_the_json_type_found(#[case] byte: u8, #[case] expected: &str) {
    assert_eq!(kind_name(byte), expected);
}

#[rstest]
fn takes_the_byte_it_expects() {
    let mut scanner = Scanner { bytes: b"[", at: 0 };
    scanner.take_byte(b'{').unwrap_err();
}

#[rstest]
fn skips_leading_whitespace() {
    let mut scanner = Scanner {
        bytes: b"  \n[1]",
        at: 0,
    };
    assert_eq!(scanner.peek(), Some(b'['));
}

#[rstest]
fn refuses_a_container_that_does_not_close() {
    let mut scanner = Scanner {
        bytes: b"[1}",
        at: 0,
    };
    scanner.skip_value().unwrap_err();
}
