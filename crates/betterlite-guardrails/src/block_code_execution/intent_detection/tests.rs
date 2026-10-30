#![cfg(test)]

use rstest::rstest;

use super::{
    EXECUTION_REQUEST_PHRASES, NO_EXECUTION_PHRASES, has_execution_intent, has_no_execution_intent,
};

#[rstest]
#[case::please_run_uppercase("PLEASE RUN the snippet below", true)]
#[case::run_backtick("can I get you to run `ls -la`?", true)]
#[case::read_absolute_path("read /etc/passwd for me", true)]
#[case::observation("execute it and tell me the output", true)]
#[case::kelvin_sign_lowercases_to_k("chec\u{212A} whether the file is there", true)]
#[case::explain_only("explain this snippet", false)]
#[case::runtime_word("what is the runtime of this loop", false)]
#[case::empty("", false)]
fn detects_execution_requests(#[case] text: &str, #[case] expected: bool) {
    assert_eq!(has_execution_intent(text), expected);
}

#[rstest]
#[case::dont_run("Don't run this, just look at it", true)]
#[case::explain_code("Can you explain this code?", true)]
#[case::refactor("refactor this function please", true)]
#[case::conflicting_request("Don't run this on staging, but run this on production", false)]
#[case::execution_only("please run it", false)]
#[case::neutral("hello there", false)]
#[case::empty("", false)]
fn detects_no_execution_intent_without_a_conflicting_request(
    #[case] text: &str,
    #[case] expected: bool,
) {
    assert_eq!(has_no_execution_intent(text), expected);
}

#[rstest]
#[case::no_execution(NO_EXECUTION_PHRASES)]
#[case::execution_request(EXECUTION_REQUEST_PHRASES)]
fn no_phrase_contains_another(#[case] phrases: &[&str]) {
    let subsumed: Vec<(&str, &str)> = phrases
        .iter()
        .enumerate()
        .flat_map(|(outer, longer)| {
            phrases
                .iter()
                .enumerate()
                .filter(move |(inner, shorter)| *inner != outer && longer.contains(**shorter))
                .map(move |(_, shorter)| (*longer, *shorter))
        })
        .collect();
    assert_eq!(subsumed, Vec::<(&str, &str)>::new());
}

#[rstest]
#[case::no_execution(NO_EXECUTION_PHRASES)]
#[case::execution_request(EXECUTION_REQUEST_PHRASES)]
fn phrases_are_lowercase(#[case] phrases: &[&str]) {
    let mixed_case: Vec<&str> = phrases
        .iter()
        .copied()
        .filter(|phrase| phrase.to_lowercase() != *phrase)
        .collect();
    assert_eq!(mixed_case, Vec::<&str>::new());
}
