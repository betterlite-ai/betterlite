#![cfg(test)]

use super::count_signals;
use rstest::rstest;

#[rstest]
#[case::once_per_pattern("better BETTER", vec!["better".to_owned()], 1)]
#[case::duplicate_patterns("better", vec!["better".to_owned(), "better".to_owned()], 2)]
#[case::regex("best", vec!["b.*t".to_owned()], 1)]
#[case::empty_pattern("", vec![String::new()], 1)]
#[case::no_patterns("better", Vec::new(), 0)]
fn signals(#[case] text: &str, #[case] patterns: Vec<String>, #[case] expected: usize) {
    assert_eq!(count_signals(text, &patterns).unwrap(), expected);
}

#[test]
fn invalid_pattern() {
    let error = count_signals("better", &["[".to_owned()]).unwrap_err();
    assert!(matches!(error, super::Error::Pattern { .. }));
}
