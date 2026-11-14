#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::sections::content_category::loading::CategoryLoadResult;
use betterlite_config::sections::content_category::model::{CategoryConfig, CompiledPhrasePattern};
use betterlite_config::sections::content_category::pattern::Pattern;
use betterlite_config::sections::content_filter_records::ContentFilterAction;
use betterlite_config::sections::content_filter_records::category_types::ConditionalCategoryConfig;
use rstest::rstest;

use super::{
    CategoryMatch, ContentFilterMatchingError, check_category_keywords,
    check_conditional_categories, check_phrase_patterns,
};

type KeywordEntry = (String, String, ContentFilterAction);
type KeywordMap = BTreeMap<String, KeywordEntry>;

fn category(name: &str, action: ContentFilterAction) -> CategoryConfig {
    CategoryConfig {
        category_name: name.to_owned(),
        description: String::new(),
        default_action: action,
        keywords: Vec::new(),
        exceptions: Vec::new(),
        identifier_words: Vec::new(),
        always_block_keywords: Vec::new(),
        inherit_from: None,
        additional_block_words: Vec::new(),
        phrase_patterns: Vec::new(),
    }
}

fn phrase_category(name: &str, action: ContentFilterAction, patterns: &[&str]) -> CategoryConfig {
    let mut category = category(name, action);
    for pattern in patterns {
        category.phrase_patterns.push(CompiledPhrasePattern {
            source: (*pattern).to_owned(),
            regex: Pattern::compile(pattern).unwrap(),
        });
    }
    category
}

fn conditional(
    action: ContentFilterAction,
    severity: &str,
    identifiers: &[&str],
    blocks: &[&str],
) -> ConditionalCategoryConfig {
    ConditionalCategoryConfig {
        identifier_words: identifiers.iter().map(|word| (*word).to_owned()).collect(),
        block_words: blocks.iter().map(|word| (*word).to_owned()).collect(),
        action,
        severity: severity.to_owned(),
    }
}

fn keyword_map(entries: &[(&str, &str, &str, ContentFilterAction)]) -> KeywordMap {
    entries
        .iter()
        .map(|(keyword, category, severity, action)| {
            (
                (*keyword).to_owned(),
                ((*category).to_owned(), (*severity).to_owned(), *action),
            )
        })
        .collect()
}

#[rstest]
#[case::same_sentence("the minor asked about romantic things", Some("minor + romantic"))]
#[case::separate_sentences("the minor is here. it was romantic", None)]
#[case::identifier_alone("the minor is here", None)]
#[case::block_word_alone("it was romantic", None)]
#[case::second_sentence(
    "stop. the minor asked about romantic things",
    Some("minor + romantic")
)]
fn conditional_matches_need_the_identifier_and_block_word_in_one_sentence(
    #[case] text: &str,
    #[case] expected: Option<&str>,
) {
    let mut categories = CategoryLoadResult::default();
    categories.conditional_categories.insert(
        "child_safety".to_owned(),
        conditional(
            ContentFilterAction::Block,
            "high",
            &["minor"],
            &["romantic"],
        ),
    );
    let found = check_conditional_categories(text, &[], &categories).unwrap();
    assert_eq!(
        found
            .as_ref()
            .map(|category_match| category_match.matched.as_str()),
        expected
    );
}

#[test]
fn conditional_matches_report_the_category_severity_and_action() {
    let mut categories = CategoryLoadResult::default();
    categories.conditional_categories.insert(
        "child_safety".to_owned(),
        conditional(ContentFilterAction::Mask, "low", &["minor"], &["romantic"]),
    );
    assert_eq!(
        check_conditional_categories("a minor with romantic ideas", &[], &categories).unwrap(),
        Some(CategoryMatch {
            matched: "minor + romantic".to_owned(),
            category: "child_safety".to_owned(),
            severity: "low".to_owned(),
            action: ContentFilterAction::Mask,
        })
    );
}

#[rstest]
#[case::period("the minor. romantic ideas", None)]
#[case::exclamation("the minor! romantic ideas", None)]
#[case::question("the minor? romantic ideas", None)]
#[case::trailing_terminator("the minor with romantic ideas.", Some("minor + romantic"))]
#[case::repeated_terminators("the minor.. romantic ideas", None)]
fn conditional_sentences_split_on_common_terminators(
    #[case] text: &str,
    #[case] expected: Option<&str>,
) {
    let mut categories = CategoryLoadResult::default();
    categories.conditional_categories.insert(
        "child_safety".to_owned(),
        conditional(
            ContentFilterAction::Block,
            "high",
            &["minor"],
            &["romantic"],
        ),
    );
    let found = check_conditional_categories(text, &[], &categories).unwrap();
    assert_eq!(
        found
            .as_ref()
            .map(|category_match| category_match.matched.as_str()),
        expected
    );
}

#[test]
fn conditional_matching_ignores_case() {
    let mut categories = CategoryLoadResult::default();
    categories.conditional_categories.insert(
        "child_safety".to_owned(),
        conditional(
            ContentFilterAction::Block,
            "high",
            &["minor"],
            &["romantic"],
        ),
    );
    let found =
        check_conditional_categories("A MINOR WITH ROMANTIC IDEAS", &[], &categories).unwrap();
    assert_eq!(
        found.map(|category_match| category_match.matched),
        Some("minor + romantic".to_owned())
    );
}

#[test]
fn conditional_identifier_words_match_inside_longer_words() {
    let mut categories = CategoryLoadResult::default();
    categories.conditional_categories.insert(
        "child_safety".to_owned(),
        conditional(
            ContentFilterAction::Block,
            "high",
            &["minor"],
            &["romantic"],
        ),
    );
    let found =
        check_conditional_categories("minors with romantic ideas", &[], &categories).unwrap();
    assert_eq!(
        found.map(|category_match| category_match.matched),
        Some("minor + romantic".to_owned())
    );
}

#[rstest]
#[case::apart("a minor with aromantic ideas", None)]
#[case::whole_word("a minor with romantic ideas", Some("minor + romantic"))]
fn conditional_single_word_block_words_use_word_boundaries(
    #[case] text: &str,
    #[case] expected: Option<&str>,
) {
    let mut categories = CategoryLoadResult::default();
    categories.conditional_categories.insert(
        "child_safety".to_owned(),
        conditional(
            ContentFilterAction::Block,
            "high",
            &["minor"],
            &["romantic"],
        ),
    );
    let found = check_conditional_categories(text, &[], &categories).unwrap();
    assert_eq!(
        found
            .as_ref()
            .map(|category_match| category_match.matched.as_str()),
        expected
    );
}

#[rstest]
#[case::whole_phrase("a minor with romantic ideas here", Some("minor + romantic ideas"))]
#[case::inside_a_longer_word("a minor with aromantic ideas here", Some("minor + romantic ideas"))]
#[case::partial_phrase("a minor with romantic thoughts", None)]
fn conditional_multi_word_block_words_use_substring_matching(
    #[case] text: &str,
    #[case] expected: Option<&str>,
) {
    let mut categories = CategoryLoadResult::default();
    categories.conditional_categories.insert(
        "child_safety".to_owned(),
        conditional(
            ContentFilterAction::Block,
            "high",
            &["minor"],
            &["romantic ideas"],
        ),
    );
    let found = check_conditional_categories(text, &[], &categories).unwrap();
    assert_eq!(
        found
            .as_ref()
            .map(|category_match| category_match.matched.as_str()),
        expected
    );
}

#[rstest]
#[case::literal_asterisk("a minor with r*m", Some("minor + r*m"))]
#[case::no_wildcard("a minor with rem", None)]
fn conditional_block_words_treat_asterisks_literally(
    #[case] text: &str,
    #[case] expected: Option<&str>,
) {
    let mut categories = CategoryLoadResult::default();
    categories.conditional_categories.insert(
        "child_safety".to_owned(),
        conditional(ContentFilterAction::Block, "high", &["minor"], &["r*m"]),
    );
    let found = check_conditional_categories(text, &[], &categories).unwrap();
    assert_eq!(
        found
            .as_ref()
            .map(|category_match| category_match.matched.as_str()),
        expected
    );
}

#[test]
fn conditional_block_word_boundaries_use_the_regex_crate_word_set() {
    let mut categories = CategoryLoadResult::default();
    categories.conditional_categories.insert(
        "child_safety".to_owned(),
        conditional(
            ContentFilterAction::Block,
            "high",
            &["minor"],
            &["romantic"],
        ),
    );
    assert_eq!(
        check_conditional_categories("a minor with romantic\u{200d} ideas", &[], &categories)
            .unwrap(),
        None
    );
}

#[test]
fn conditional_matches_report_the_first_block_word_found() {
    let mut categories = CategoryLoadResult::default();
    categories.conditional_categories.insert(
        "child_safety".to_owned(),
        conditional(
            ContentFilterAction::Block,
            "high",
            &["minor"],
            &["romantic", "kiss"],
        ),
    );
    let found = check_conditional_categories("a minor with a kiss", &[], &categories).unwrap();
    assert_eq!(
        found.map(|category_match| category_match.matched),
        Some("minor + kiss".to_owned())
    );
}

#[test]
fn conditional_category_exceptions_skip_that_category_and_keep_looking() {
    let mut exempt = category("alpha", ContentFilterAction::Block);
    exempt.exceptions.push("safe".to_owned());
    let mut categories = CategoryLoadResult::default();
    categories
        .loaded_categories
        .insert("alpha".to_owned(), exempt);
    categories.conditional_categories.insert(
        "alpha".to_owned(),
        conditional(
            ContentFilterAction::Block,
            "high",
            &["minor"],
            &["romantic"],
        ),
    );
    categories.conditional_categories.insert(
        "beta".to_owned(),
        conditional(ContentFilterAction::Mask, "low", &["minor"], &["romantic"]),
    );
    assert_eq!(
        check_conditional_categories("a minor with romantic ideas, SAFE", &[], &categories)
            .unwrap(),
        Some(CategoryMatch {
            matched: "minor + romantic".to_owned(),
            category: "beta".to_owned(),
            severity: "low".to_owned(),
            action: ContentFilterAction::Mask,
        })
    );
}

#[test]
fn conditional_global_exceptions_prevent_every_category() {
    let mut categories = CategoryLoadResult::default();
    categories.conditional_categories.insert(
        "child_safety".to_owned(),
        conditional(
            ContentFilterAction::Block,
            "high",
            &["minor"],
            &["romantic"],
        ),
    );
    assert_eq!(
        check_conditional_categories(
            "a minor with romantic ideas, SAFE",
            &["safe".to_owned()],
            &categories
        )
        .unwrap(),
        None
    );
}

#[test]
fn conditional_categories_without_a_loaded_entry_still_match() {
    let mut categories = CategoryLoadResult::default();
    categories.conditional_categories.insert(
        "child_safety".to_owned(),
        conditional(
            ContentFilterAction::Block,
            "high",
            &["minor"],
            &["romantic"],
        ),
    );
    assert!(
        check_conditional_categories("a minor with romantic ideas", &[], &categories)
            .unwrap()
            .is_some()
    );
}

#[test]
fn conditional_categories_are_checked_in_name_order() {
    let mut categories = CategoryLoadResult::default();
    categories.conditional_categories.insert(
        "beta".to_owned(),
        conditional(ContentFilterAction::Mask, "low", &["minor"], &["kiss"]),
    );
    categories.conditional_categories.insert(
        "alpha".to_owned(),
        conditional(
            ContentFilterAction::Block,
            "high",
            &["minor"],
            &["romantic"],
        ),
    );
    let found =
        check_conditional_categories("a minor with romantic ideas and a kiss", &[], &categories)
            .unwrap();
    assert_eq!(
        found.map(|category_match| category_match.category),
        Some("alpha".to_owned())
    );
}

#[test]
fn phrase_patterns_match_paraphrases_regardless_of_case() {
    let mut categories = CategoryLoadResult::default();
    categories.loaded_categories.insert(
        "financial_advice".to_owned(),
        phrase_category(
            "financial_advice",
            ContentFilterAction::Block,
            &["put my money to make it grow"],
        ),
    );
    assert_eq!(
        check_phrase_patterns("PLEASE PUT MY MONEY TO MAKE IT GROW", &[], &categories).unwrap(),
        Some(CategoryMatch {
            matched: "phrase: put my money to make it grow".to_owned(),
            category: "financial_advice".to_owned(),
            severity: "high".to_owned(),
            action: ContentFilterAction::Block,
        })
    );
}

#[test]
fn phrase_patterns_return_none_without_a_match() {
    let mut categories = CategoryLoadResult::default();
    categories.loaded_categories.insert(
        "financial_advice".to_owned(),
        phrase_category(
            "financial_advice",
            ContentFilterAction::Block,
            &["put my money to make it grow"],
        ),
    );
    assert_eq!(
        check_phrase_patterns("a plain question", &[], &categories).unwrap(),
        None
    );
}

#[test]
fn phrase_patterns_report_the_first_matching_source() {
    let mut categories = CategoryLoadResult::default();
    categories.loaded_categories.insert(
        "financial_advice".to_owned(),
        phrase_category(
            "financial_advice",
            ContentFilterAction::Block,
            &["money will grow", "money"],
        ),
    );
    let found = check_phrase_patterns("money will grow", &[], &categories).unwrap();
    assert_eq!(
        found.map(|category_match| category_match.matched),
        Some("phrase: money will grow".to_owned())
    );
}

#[test]
fn phrase_patterns_skip_a_source_that_does_not_match() {
    let mut categories = CategoryLoadResult::default();
    categories.loaded_categories.insert(
        "financial_advice".to_owned(),
        phrase_category(
            "financial_advice",
            ContentFilterAction::Block,
            &["banana", "money"],
        ),
    );
    let found = check_phrase_patterns("money", &[], &categories).unwrap();
    assert_eq!(
        found.map(|category_match| category_match.matched),
        Some("phrase: money".to_owned())
    );
}

#[test]
fn phrase_patterns_use_the_category_default_action() {
    let mut categories = CategoryLoadResult::default();
    categories.loaded_categories.insert(
        "financial_advice".to_owned(),
        phrase_category(
            "financial_advice",
            ContentFilterAction::Mask,
            &["put my money to make it grow"],
        ),
    );
    let found = check_phrase_patterns("put my money to make it grow", &[], &categories).unwrap();
    assert_eq!(
        found.map(|category_match| category_match.action),
        Some(ContentFilterAction::Mask)
    );
}

#[test]
fn phrase_patterns_take_the_conditional_action_and_report_high_severity() {
    let mut categories = CategoryLoadResult::default();
    categories.loaded_categories.insert(
        "child_safety".to_owned(),
        phrase_category(
            "child_safety",
            ContentFilterAction::Block,
            &["run away from home"],
        ),
    );
    categories.conditional_categories.insert(
        "child_safety".to_owned(),
        conditional(ContentFilterAction::Mask, "low", &["run"], &["home"]),
    );
    assert_eq!(
        check_phrase_patterns("run away from home", &[], &categories).unwrap(),
        Some(CategoryMatch {
            matched: "phrase: run away from home".to_owned(),
            category: "child_safety".to_owned(),
            severity: "high".to_owned(),
            action: ContentFilterAction::Mask,
        })
    );
}

#[rstest]
#[case::category_exception("PUT MY MONEY TO MAKE IT GROW, SAFE", None)]
#[case::without_the_exception(
    "PUT MY MONEY TO MAKE IT GROW",
    Some("phrase: put my money to make it grow")
)]
fn phrase_patterns_skip_a_category_whose_exception_is_present(
    #[case] text: &str,
    #[case] expected: Option<&str>,
) {
    let mut exempt = phrase_category(
        "financial_advice",
        ContentFilterAction::Block,
        &["put my money to make it grow"],
    );
    exempt.exceptions.push("safe".to_owned());
    let mut categories = CategoryLoadResult::default();
    categories
        .loaded_categories
        .insert("financial_advice".to_owned(), exempt);
    let found = check_phrase_patterns(text, &[], &categories).unwrap();
    assert_eq!(
        found
            .as_ref()
            .map(|category_match| category_match.matched.as_str()),
        expected
    );
}

#[test]
fn phrase_patterns_stop_on_a_global_exception() {
    let mut categories = CategoryLoadResult::default();
    categories.loaded_categories.insert(
        "financial_advice".to_owned(),
        phrase_category(
            "financial_advice",
            ContentFilterAction::Block,
            &["put my money to make it grow"],
        ),
    );
    assert_eq!(
        check_phrase_patterns(
            "PUT MY MONEY TO MAKE IT GROW, SAFE",
            &["safe".to_owned()],
            &categories
        )
        .unwrap(),
        None
    );
}

#[test]
fn phrase_patterns_skip_categories_without_patterns() {
    let mut categories = CategoryLoadResult::default();
    categories.loaded_categories.insert(
        "alpha".to_owned(),
        category("alpha", ContentFilterAction::Block),
    );
    categories.loaded_categories.insert(
        "beta".to_owned(),
        phrase_category("beta", ContentFilterAction::Mask, &["money"]),
    );
    let found = check_phrase_patterns("money", &[], &categories).unwrap();
    assert_eq!(
        found.map(|category_match| category_match.category),
        Some("beta".to_owned())
    );
}

#[test]
fn phrase_patterns_are_checked_in_category_name_order() {
    let mut categories = CategoryLoadResult::default();
    categories.loaded_categories.insert(
        "beta".to_owned(),
        phrase_category("beta", ContentFilterAction::Mask, &["money"]),
    );
    categories.loaded_categories.insert(
        "alpha".to_owned(),
        phrase_category("alpha", ContentFilterAction::Block, &["money"]),
    );
    let found = check_phrase_patterns("money", &[], &categories).unwrap();
    assert_eq!(
        found.map(|category_match| category_match.category),
        Some("alpha".to_owned())
    );
}

#[test]
fn phrase_patterns_surface_a_pattern_matching_error() {
    let mut risky = category("risky", ContentFilterAction::Block);
    risky.phrase_patterns.push(CompiledPhrasePattern {
        source: "(a|aa)+$".to_owned(),
        regex: Pattern::compile("(a|aa)+$").unwrap(),
    });
    let mut categories = CategoryLoadResult::default();
    categories
        .loaded_categories
        .insert("risky".to_owned(), risky);
    let text = format!("{}b", "a".repeat(64));
    assert!(matches!(
        check_phrase_patterns(&text, &[], &categories),
        Err(ContentFilterMatchingError::PhrasePattern(_))
    ));
}

#[test]
fn always_block_keywords_are_checked_before_regular_keywords() {
    let mut categories = CategoryLoadResult::default();
    categories
        .always_block_category_keywords
        .extend(keyword_map(&[(
            "zebra",
            "always",
            "high",
            ContentFilterAction::Block,
        )]));
    categories.category_keywords.extend(keyword_map(&[(
        "apple",
        "regular",
        "low",
        ContentFilterAction::Mask,
    )]));
    assert_eq!(
        check_category_keywords("an APPLE next to a ZEBRA", &[], &categories).unwrap(),
        Some(CategoryMatch {
            matched: "zebra".to_owned(),
            category: "always".to_owned(),
            severity: "high".to_owned(),
            action: ContentFilterAction::Block,
        })
    );
}

#[test]
fn always_block_keywords_only_check_the_global_exceptions() {
    let mut exempt = category("always", ContentFilterAction::Block);
    exempt.exceptions.push("safe".to_owned());
    let mut categories = CategoryLoadResult::default();
    categories
        .loaded_categories
        .insert("always".to_owned(), exempt);
    categories
        .always_block_category_keywords
        .extend(keyword_map(&[(
            "zebra",
            "always",
            "high",
            ContentFilterAction::Block,
        )]));
    assert!(
        check_category_keywords("a zebra is safe", &[], &categories)
            .unwrap()
            .is_some()
    );
}

#[test]
fn regular_keywords_skip_a_category_whose_exception_is_present() {
    let mut exempt = category("food", ContentFilterAction::Mask);
    exempt.exceptions.push("safe".to_owned());
    let mut categories = CategoryLoadResult::default();
    categories
        .loaded_categories
        .insert("food".to_owned(), exempt);
    categories.category_keywords.extend(keyword_map(&[
        ("apple", "food", "low", ContentFilterAction::Mask),
        ("danger", "risk", "high", ContentFilterAction::Block),
    ]));
    assert_eq!(
        check_category_keywords("apple and danger are SAFE", &[], &categories).unwrap(),
        Some(CategoryMatch {
            matched: "danger".to_owned(),
            category: "risk".to_owned(),
            severity: "high".to_owned(),
            action: ContentFilterAction::Block,
        })
    );
}

#[test]
fn regular_keywords_return_none_when_only_the_exempt_category_matches() {
    let mut exempt = category("food", ContentFilterAction::Mask);
    exempt.exceptions.push("safe".to_owned());
    let mut categories = CategoryLoadResult::default();
    categories
        .loaded_categories
        .insert("food".to_owned(), exempt);
    categories.category_keywords.extend(keyword_map(&[(
        "apple",
        "food",
        "low",
        ContentFilterAction::Mask,
    )]));
    assert_eq!(
        check_category_keywords("apple is SAFE", &[], &categories).unwrap(),
        None
    );
}

#[test]
fn category_keywords_stop_on_a_global_exception() {
    let mut categories = CategoryLoadResult::default();
    categories
        .always_block_category_keywords
        .extend(keyword_map(&[(
            "zebra",
            "always",
            "high",
            ContentFilterAction::Block,
        )]));
    assert_eq!(
        check_category_keywords("a zebra is SAFE", &["safe".to_owned()], &categories).unwrap(),
        None
    );
}

#[rstest]
#[case::inside_a_word("i recommend this", None)]
#[case::whole_word("ask the men", Some("men"))]
#[case::punctuation_neighbour("ask the men.", Some("men"))]
fn single_word_keywords_use_word_boundaries(#[case] text: &str, #[case] expected: Option<&str>) {
    let mut categories = CategoryLoadResult::default();
    categories.category_keywords.extend(keyword_map(&[(
        "men",
        "gender",
        "medium",
        ContentFilterAction::Block,
    )]));
    let found = check_category_keywords(text, &[], &categories).unwrap();
    assert_eq!(
        found
            .as_ref()
            .map(|category_match| category_match.matched.as_str()),
        expected
    );
}

#[rstest]
#[case::whole_phrase("an ice cream cone", Some("ice cream"))]
#[case::inside_a_longer_word("a nice cream", Some("ice cream"))]
#[case::partial_phrase("i like ice", None)]
fn multi_word_keywords_use_substring_matching(#[case] text: &str, #[case] expected: Option<&str>) {
    let mut categories = CategoryLoadResult::default();
    categories.category_keywords.extend(keyword_map(&[(
        "ice cream",
        "food",
        "medium",
        ContentFilterAction::Block,
    )]));
    let found = check_category_keywords(text, &[], &categories).unwrap();
    assert_eq!(
        found
            .as_ref()
            .map(|category_match| category_match.matched.as_str()),
        expected
    );
}

#[rstest]
#[case::no_extra_characters("fuck", true)]
#[case::one_extra_character_per_star("fuuck", true)]
#[case::two_extra_characters("fuuuck", false)]
#[case::missing_letter("fxck", false)]
fn keywords_treat_asterisks_as_a_zero_or_one_character_wildcard(
    #[case] text: &str,
    #[case] matched: bool,
) {
    let mut categories = CategoryLoadResult::default();
    categories.category_keywords.extend(keyword_map(&[(
        "fu*c*k",
        "profanity",
        "high",
        ContentFilterAction::Block,
    )]));
    assert_eq!(
        check_category_keywords(text, &[], &categories)
            .unwrap()
            .is_some(),
        matched
    );
}

#[rstest]
#[case::wildcard_matches_zero_characters("child  abuse", true)]
#[case::wildcard_matches_one_character("child x abuse", true)]
#[case::no_room_for_the_wildcard("child abuse", false)]
fn multi_word_keywords_apply_asterisk_wildcards(#[case] text: &str, #[case] matched: bool) {
    let mut categories = CategoryLoadResult::default();
    categories.category_keywords.extend(keyword_map(&[(
        "child * abuse",
        "child_safety",
        "high",
        ContentFilterAction::Block,
    )]));
    assert_eq!(
        check_category_keywords(text, &[], &categories)
            .unwrap()
            .is_some(),
        matched
    );
}

#[test]
fn keywords_match_regardless_of_case_and_report_the_stored_form() {
    let mut categories = CategoryLoadResult::default();
    categories.category_keywords.extend(keyword_map(&[(
        "alert",
        "operational",
        "medium",
        ContentFilterAction::Mask,
    )]));
    let found = check_category_keywords("THIS IS AN ALERT", &[], &categories).unwrap();
    assert_eq!(
        found.map(|category_match| category_match.matched),
        Some("alert".to_owned())
    );
}

#[test]
fn category_keywords_are_checked_in_keyword_order() {
    let mut categories = CategoryLoadResult::default();
    categories.category_keywords.extend(keyword_map(&[
        ("beta", "second", "low", ContentFilterAction::Mask),
        ("alpha", "first", "high", ContentFilterAction::Block),
    ]));
    let found = check_category_keywords("alpha and beta", &[], &categories).unwrap();
    assert_eq!(
        found.map(|category_match| category_match.matched),
        Some("alpha".to_owned())
    );
}

#[test]
fn always_block_keywords_are_checked_in_keyword_order() {
    let mut categories = CategoryLoadResult::default();
    categories
        .always_block_category_keywords
        .extend(keyword_map(&[
            ("beta", "second", "low", ContentFilterAction::Mask),
            ("alpha", "first", "high", ContentFilterAction::Block),
        ]));
    let found = check_category_keywords("alpha and beta", &[], &categories).unwrap();
    assert_eq!(
        found.map(|category_match| category_match.matched),
        Some("alpha".to_owned())
    );
}

#[test]
fn single_word_keyword_boundaries_use_the_regex_crate_word_set() {
    let mut categories = CategoryLoadResult::default();
    categories.category_keywords.extend(keyword_map(&[(
        "men",
        "gender",
        "medium",
        ContentFilterAction::Block,
    )]));
    assert_eq!(
        check_category_keywords("men\u{200d} here", &[], &categories).unwrap(),
        None
    );
    assert!(
        check_category_keywords("men\u{b2} here", &[], &categories)
            .unwrap()
            .is_some()
    );
}

#[test]
fn category_keywords_return_none_without_a_match() {
    let mut categories = CategoryLoadResult::default();
    categories.category_keywords.extend(keyword_map(&[(
        "alpha",
        "first",
        "high",
        ContentFilterAction::Block,
    )]));
    assert_eq!(
        check_category_keywords("nothing here", &[], &categories).unwrap(),
        None
    );
}
