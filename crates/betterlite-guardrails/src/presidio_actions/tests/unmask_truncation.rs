#![cfg(test)]

use indexmap::IndexMap;

use crate::presidio_actions::tokens::unmask_pii_text;

fn token_map(token: &str, original: &str) -> IndexMap<String, String> {
    IndexMap::from([(token.to_owned(), original.to_owned())])
}

#[test]
fn a_truncated_token_at_the_end_is_replaced_when_the_overlap_is_long_enough() {
    let map = token_map("<PERSON_1>", "Alice");
    assert_eq!(unmask_pii_text("Hello <PERS", &map), "Hello Alice");
    assert_eq!(unmask_pii_text("Hello <PERSON_1", &map), "Hello Alice");
}

#[test]
fn a_truncated_token_shorter_than_the_minimum_overlap_is_kept() {
    let map = token_map("<PERSON_1>", "Alice");
    assert_eq!(unmask_pii_text("Hello <PER", &map), "Hello <PER");
}

#[test]
fn a_token_prefix_that_is_not_at_the_end_is_never_matched() {
    let map = token_map("<PERSON_1>", "Alice");
    assert_eq!(unmask_pii_text("<PERS is here", &map), "<PERS is here");
}

#[test]
fn the_minimum_overlap_is_capped_at_20_characters() {
    let token = format!("<{}_1>", "A".repeat(50));
    let map = token_map(&token, "Long");
    let nineteen: String = token.chars().take(19).collect();
    let twenty: String = token.chars().take(20).collect();
    assert_eq!(
        unmask_pii_text(&format!("value {nineteen}"), &map),
        format!("value {nineteen}")
    );
    assert_eq!(
        unmask_pii_text(&format!("value {twenty}"), &map),
        "value Long"
    );
}

#[test]
fn a_short_token_uses_half_its_length_as_the_minimum_overlap() {
    let map = token_map("<A_1>", "x");
    assert_eq!(unmask_pii_text("value <A", &map), "value x");
    assert_eq!(unmask_pii_text("value <", &map), "value <");
}

#[test]
fn the_truncation_fallback_only_runs_when_no_full_token_is_present() {
    let map = token_map("<PERSON_1>", "Alice");
    assert_eq!(
        unmask_pii_text("Hi <PERSON_1> and <PERS", &map),
        "Hi Alice and <PERS"
    );
}
