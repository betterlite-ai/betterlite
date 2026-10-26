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
