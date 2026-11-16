#![cfg(test)]

use crate::transcription::encoding::quote_plus;

#[test]
fn quote_plus_matches_python_urlencode() {
    assert_eq!(quote_plus("en-US"), "en-US");
    assert_eq!(quote_plus("zh CN"), "zh+CN");
    assert_eq!(quote_plus("a*b~c"), "a%2Ab~c");
    assert_eq!(quote_plus("2025-02-01-preview"), "2025-02-01-preview");
}
