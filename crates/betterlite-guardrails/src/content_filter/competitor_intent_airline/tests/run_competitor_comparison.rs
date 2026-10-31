#![cfg(test)]

use super::shared::airline_checker;

#[test]
fn comparison_language_with_resolved_competitor_is_competitor_comparison() {
    let result = airline_checker().run("Qatar Airways is better");
    assert_eq!(result.intent.as_deref(), Some("competitor_comparison"));
    assert_eq!(result.action_hint.as_deref(), Some("allow"));
}
