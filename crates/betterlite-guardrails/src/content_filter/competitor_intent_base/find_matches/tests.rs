#![cfg(test)]

use super::BaseCompetitorIntentChecker;
use super::matching::find_matches;
use super::tests::config;

#[test]
fn canonical_and_alias() {
    let checker = BaseCompetitorIntentChecker::new(&config()).unwrap();
    let matches = find_matches(&checker, "Acme and AC");
    assert_eq!(matches.len(), 2);
    assert!(matches.iter().all(|(_, canonical, _)| canonical == "acme"));
}

#[test]
fn find_matches_deduplicates() {
    let mut configured = config();
    configured.competitors.push("Acme".to_owned());
    configured.competitor_aliases.insert(
        "acme".to_owned(),
        vec![
            "AC".to_owned(),
            " ac ".to_owned(),
            "ace".to_owned(),
            " ACME ".to_owned(),
        ],
    );
    let checker = BaseCompetitorIntentChecker::new(&configured).unwrap();
    assert_eq!(
        find_matches(&checker, "@cme AC ac ace acmeish"),
        vec![
            ("ac".to_owned(), "acme".to_owned(), false),
            ("ace".to_owned(), "acme".to_owned(), false),
            ("acme".to_owned(), "acme".to_owned(), true),
        ]
    );
}

#[test]
fn find_matches_unicode() {
    let mut configured = config();
    configured.competitors = vec!["\u{d1}o\u{f1}o Airways".to_owned()];
    let checker = BaseCompetitorIntentChecker::new(&configured).unwrap();
    assert_eq!(find_matches(&checker, "\u{d1}O\u{d1}O AIRWAYS").len(), 1);
}
