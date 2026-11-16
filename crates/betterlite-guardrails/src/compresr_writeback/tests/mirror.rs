use std::collections::BTreeMap;

use rstest::rstest;

use super::support::{TOOL_CONVERSATION, TWO_IDENTICAL, conversation, guardrail, reply, serve};
use crate::compresr_writeback::apply_compression;
use crate::compresr_writeback::mirror::mirror_texts_channel;
use crate::compresr_writeback::results::Replacements;
use crate::generic_guardrail_contracts::GuardrailInputType;

fn texts(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| (*item).to_owned()).collect()
}

fn compressed(pairs: &[(&str, &str)]) -> Replacements {
    let mut replacements = Replacements::default();
    for (original, compression) in pairs {
        replacements.note(original, compression);
    }
    replacements
}

#[rstest]
#[case::once_each(&[("a", "A")], &["a", "b"], Some(&["A", "b"][..]))]
#[case::both_occurrences(&[("a", "A"), ("a", "A")], &["a", "b", "a"], Some(&["A", "b", "A"][..]))]
#[case::one_compression_two_occurrences(&[("a", "A")], &["a", "a"], None)]
#[case::two_compressions_one_occurrence(&[("a", "A"), ("a", "A")], &["a"], None)]
#[case::different_compressions_are_ambiguous(&[("a", "A"), ("a", "B")], &["a", "a"], None)]
#[case::absent_text(&[("a", "A")], &["b"], None)]
#[case::only_the_safe_text(&[("a", "A"), ("b", "B"), ("b", "C")], &["a", "b", "b"], Some(&["A", "b", "b"][..]))]
fn replaces_only_what_is_safe_mirror(
    #[case] pairs: &[(&str, &str)],
    #[case] input: &[&str],
    #[case] expected: Option<&[&str]>,
) {
    let input_texts = texts(input);
    let mirrored = mirror_texts_channel(Some(input_texts.as_slice()), &compressed(pairs));
    assert_eq!(mirrored, expected.map(texts));
}

#[test]
fn needs_texts_and_replacements_mirror() {
    assert_eq!(mirror_texts_channel(None, &compressed(&[("a", "A")])), None);
    let input = texts(&["a"]);
    let nothing = Replacements::default();
    assert_eq!(mirror_texts_channel(Some(input.as_slice()), &nothing), None);
}

const TEXTS: &str = r#","texts":["long tool output one","long tool output one"]"#;

#[rstest]
#[case::different_compressions(&[("first", 10, 4), ("second", 10, 4)], &["long tool output one", "long tool output one"])]
#[case::the_same_compression(&[("same", 10, 4), ("same", 10, 4)], &["same", "same"])]
#[tokio::test]
async fn identical_text_with_two_queries_ambiguous(
    #[case] results: &[(&str, i64, i64)],
    #[case] expected: &[&str],
) {
    let server = serve(200, &reply(results)).await;
    let guardrail = guardrail(&server.base, "      enable_retrieval: false\n");
    let output = apply_compression(
        &guardrail,
        conversation(TWO_IDENTICAL, TEXTS),
        &mut BTreeMap::new(),
        GuardrailInputType::Request,
        None,
    )
    .await
    .unwrap();
    assert_eq!(output.texts, Some(texts(expected)));
}

#[tokio::test]
async fn texts_stay_when_nothing_compresses_mirror() {
    let server = serve(200, &reply(&[("long tool output one", 10, 10)])).await;
    let guardrail = guardrail(&server.base, "      enable_retrieval: false\n");
    let output = apply_compression(
        &guardrail,
        conversation(TOOL_CONVERSATION, r#","texts":["long tool output one"]"#),
        &mut BTreeMap::new(),
        GuardrailInputType::Request,
        None,
    )
    .await
    .unwrap();
    assert_eq!(output.texts, Some(texts(&["long tool output one"])));
}
