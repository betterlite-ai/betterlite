#![cfg(test)]

use super::shared::airline_checker;

#[test]
fn resolved_competitor_without_comparison_is_possible_competitor_comparison() {
    assert_eq!(
        airline_checker()
            .run("Qatar airline is available")
            .intent
            .as_deref(),
        Some("possible_competitor_comparison")
    );
}
