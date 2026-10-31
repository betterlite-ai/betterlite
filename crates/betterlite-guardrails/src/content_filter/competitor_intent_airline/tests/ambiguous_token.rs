#![cfg(test)]

use super::shared::airline_checker;

#[rstest::rstest]
#[case::travel_context("flight to Qatar", "other")]
#[case::airline_context("Qatar Airways", "possible_competitor_comparison")]
fn ambiguous_competitor_location_token_resolves_by_context(
    #[case] text: &str,
    #[case] intent: &str,
) {
    assert_eq!(airline_checker().run(text).intent.as_deref(), Some(intent));
}
