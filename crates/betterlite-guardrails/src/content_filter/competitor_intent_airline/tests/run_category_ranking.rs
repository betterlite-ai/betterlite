#![cfg(test)]

use super::shared::airline_checker;

#[test]
fn comparison_and_domain_language_without_competitors_is_category_ranking() {
    assert_eq!(
        airline_checker().run("best airline").intent.as_deref(),
        Some("category_ranking")
    );
}
