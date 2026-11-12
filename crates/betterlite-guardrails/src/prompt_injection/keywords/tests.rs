#![cfg(test)]

use super::generate_injection_keywords;

#[test]
fn generates_litellms_376_keywords() {
    let keywords = generate_injection_keywords();
    assert_eq!(keywords.len(), 376);
    assert_eq!(keywords[0], "ignore and start over");
    assert_eq!(keywords[4], "ignore prior and start over");
    assert_eq!(keywords[296], "do not follow");
    assert_eq!(keywords[375], "do not obey initial and start from scratch");
}

#[test]
fn drops_phrases_with_two_or_fewer_words() {
    let keywords = generate_injection_keywords();
    assert!(!keywords.iter().any(|keyword| keyword == "ignore previous"));
    assert!(!keywords.iter().any(|keyword| keyword == "ignore"));
    assert!(
        keywords
            .iter()
            .any(|keyword| keyword == "ignore prior and start over")
    );
    assert!(keywords.iter().any(|keyword| keyword == "do not follow"));
}

#[test]
fn every_keyword_is_lowercase_with_more_than_two_words() {
    let keywords = generate_injection_keywords();
    assert!(
        keywords
            .iter()
            .all(|keyword| keyword.as_str() == keyword.to_lowercase())
    );
    assert!(
        keywords
            .iter()
            .all(|keyword| keyword.split_whitespace().count() > 2)
    );
}

#[test]
fn the_keyword_list_has_no_duplicates() {
    let mut keywords = generate_injection_keywords();
    keywords.sort_unstable();
    keywords.dedup();
    assert_eq!(keywords.len(), 376);
}
