#![cfg(test)]

use super::shared::airline_checker;

#[test]
fn plain_text_without_competitor_tokens_is_other() {
    assert_eq!(
        airline_checker().run("hello").intent.as_deref(),
        Some("other")
    );
}
