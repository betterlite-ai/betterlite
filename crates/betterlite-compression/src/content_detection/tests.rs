#![cfg(test)]

use super::*;

#[test]
fn detects_json_code_and_text() {
    assert_eq!(detect_content_type("{\"a\": 1}"), "json");
    assert_eq!(
        detect_content_type("fn main() {}\nlet x = 1;\nconst y = 2;"),
        "code"
    );
    assert_eq!(detect_content_type("a calm sentence"), "text");
}

#[test]
fn distinguishes_invalid_json_and_indentation_thresholds() {
    assert_eq!(detect_content_type("{not json}"), "text");
    assert_eq!(
        detect_content_type("one\n    two\nthree\nfour\nfive\nsix"),
        "text"
    );
    assert_eq!(
        detect_content_type("one\n    two\n    three\nfour\nfive\nsix"),
        "code"
    );
}
