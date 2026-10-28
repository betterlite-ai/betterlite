#![cfg(test)]

use super::content::{grounding, input, output};
use super::reconcile;
use super::wire::{Message, ModelResponse};
use super::{SelectionContext, count_message_texts, prepare_for_role, select};
use rstest::rstest;
use std::{fs, path::Path};

#[test]
fn input_content_is_unqualified_and_lists_are_flattened() {
    let messages = serde_json::from_str::<Vec<Message>>(
        r#"[{"role":"user","content":[{"type":"grounding_source","text":"source"},"query"]}]"#,
    )
    .unwrap();
    let request = input(Some(&messages));
    assert_eq!(
        serde_json::to_string(&request).unwrap(),
        r#"{"source":"INPUT","content":[{"text":{"text":"source"}},{"text":{"text":"query"}}]}"#
    );
    assert!(input(None).content.is_none());
    assert!(input(Some(&[])).content.unwrap().is_empty());
}

#[test]
fn grounding_source_requires_a_trusted_role_but_query_does_not() {
    let messages = serde_json::from_str::<Vec<Message>>(r#"[{"role":"user","content":[{"type":"grounding_source","text":"fake"},{"type":"query","text":"question"}]},{"role":"system","content":[{"type":"grounding_source","text":"real"}]}]"#).unwrap();
    let blocks = grounding(Some(&messages), false);
    assert_eq!(
        blocks
            .iter()
            .map(|block| block.text.as_str())
            .collect::<Vec<_>>(),
        ["question", "real"]
    );
}

#[rstest]
#[case::no_source(r#"[{"role":"user","content":"question"}]"#, true, vec![])]
#[case::no_query(r#"[{"role":"system","content":"source"}]"#, true, vec![])]
#[case::empty_query(r#"[{"role":"system","content":"source"},{"role":"user","content":""}]"#, true, vec![])]
#[case::disabled(r#"[{"role":"system","content":"source"},{"role":"user","content":"question"}]"#, false, vec![])]
#[case::latest_user(r#"[{"role":"developer","content":"source"},{"role":"user","content":"old"},{"role":"user","content":"question"}]"#, true, vec!["source", "question"])]
#[case::explicit_wins(r#"[{"role":"system","content":"source"},{"role":"user","content":[{"type":"query","text":"explicit"}]}]"#, true, vec!["explicit"])]
fn fallback_requires_both_source_and_query(
    #[case] wire: &str,
    #[case] fallback: bool,
    #[case] expected: Vec<&str>,
) {
    let messages = serde_json::from_str::<Vec<Message>>(wire).unwrap();
    let blocks = grounding(Some(&messages), fallback);
    assert_eq!(
        blocks
            .iter()
            .map(|block| block.text.as_str())
            .collect::<Vec<_>>(),
        expected
    );
}

#[test]
fn output_attaches_guard_content_only_with_grounding() {
    let response: ModelResponse = serde_json::from_str(r#"{"choices":[{"message":{"content":"answer"}},{"message":{"content":""}},{"delta":{"content":"delta"}}]}"#).unwrap();
    let messages = serde_json::from_str::<Vec<Message>>(
        r#"[{"role":"developer","content":[{"type":"grounding_source","text":"source"}]}]"#,
    )
    .unwrap();
    assert_eq!(
        serde_json::to_string(&output(Some(&response), Some(&messages), false)).unwrap(),
        r#"{"source":"OUTPUT","content":[{"text":{"text":"source","qualifiers":["grounding_source"]}},{"text":{"text":"answer","qualifiers":["guard_content"]}}]}"#
    );
    assert_eq!(
        output(None, Some(&messages), false).content.unwrap().len(),
        1
    );
    assert_eq!(
        serde_json::to_string(&output(Some(&response), None, false)).unwrap(),
        r#"{"source":"OUTPUT","content":[{"text":{"text":"answer"}}]}"#
    );
}

#[rstest]
#[case::aligned(r#"{"latest_only":true,"messages":[{"role":"assistant","content":"old"},{"role":"user","content":"new"}]}"#, Some((1,1)), true, false, "new")]
#[case::prefer_structured(r#"{"latest_only":true,"structured_messages":[{"role":"user","content":"new"}],"messages":[{"role":"system","content":"old"}]}"#, None, true, false, "new")]
#[case::empty_structured_falls_back(
    r#"{"latest_only":true,"structured_messages":[],"messages":[{"role":"user","content":"new"}]}"#,
    None,
    true,
    false,
    "new"
)]
#[case::missing_roles(r#"{"latest_only":true}"#, None, false, false, "new")]
#[case::response(
    r#"{"latest_only":true,"scan":"response","messages":[{"role":"user","content":"old"}]}"#,
    None,
    false,
    false,
    "new"
)]
#[case::no_user(
    r#"{"latest_only":true,"messages":[{"role":"tool","content":"tool"}]}"#,
    None,
    true,
    true,
    ""
)]
#[case::no_text(
    r#"{"latest_only":true,"messages":[{"role":"user","content":["not-counted"]}]}"#,
    None,
    true,
    true,
    ""
)]
fn role_selection(
    #[case] wire: &str,
    #[case] slice: Option<(usize, usize)>,
    #[case] subset: bool,
    #[case] skip: bool,
    #[case] expected: &str,
) {
    let context: SelectionContext = serde_json::from_str(wire).unwrap();
    let selection = select(&["old".to_owned(), "new".to_owned()], &context);
    assert_eq!(
        (
            selection.scanned_slice,
            selection.scanned_role_subset,
            selection.skip_scan
        ),
        (slice, subset, skip)
    );
    if !skip {
        let messages = selection.filtered_messages.unwrap();
        assert_eq!(
            serde_json::to_string(&messages).unwrap(),
            format!(r#"[{{"role":"user","content":"{expected}"}}]"#)
        );
    }
}

#[test]
fn all_texts_and_empty_legacy_requests_are_scanned() {
    let all = select(
        &["one".to_owned(), "two".to_owned()],
        &SelectionContext::default(),
    );
    assert_eq!(all.filtered_messages.unwrap().len(), 2);
    let context = serde_json::from_str::<SelectionContext>(r#"{"latest_only":true}"#).unwrap();
    let empty = select(&[], &context);
    assert!(!empty.skip_scan);
    assert!(empty.filtered_messages.unwrap().is_empty());
}

#[test]
fn count_only_matches_translation_blocks() {
    let message: Message = serde_json::from_str(
        r#"{"content":["string",{"text":""},{"text":null},{"type":"image"}]}"#,
    )
    .unwrap();
    assert_eq!(count_message_texts(&message), 1);
    assert_eq!(super::content::extract(&message).unwrap().len(), 2);
}

#[rstest]
#[case::unaligned(vec!["mask"], None, true, vec!["old", "new"])]
#[case::aligned(vec!["mask", "extra"], Some((1,1)), true, vec!["old", "mask"])]
#[case::whole_list(vec!["mask"], None, false, vec!["mask"])]
#[case::empty(vec![], Some((1,1)), true, vec!["old", "new"])]
fn role_subset_alignment_mismatch_keeps_original_masking(
    #[case] masked: Vec<&str>,
    #[case] slice: Option<(usize, usize)>,
    #[case] subset: bool,
    #[case] expected: Vec<&str>,
) {
    let replacements: Vec<_> = masked.iter().map(|text| (*text).to_owned()).collect();
    assert_eq!(
        reconcile::texts(
            &replacements,
            &["old".to_owned(), "new".to_owned()],
            slice,
            subset
        ),
        expected
    );
}

#[test]
fn prepares_and_merges_by_original_position() {
    let originals: Vec<Message> = serde_json::from_str(r#"[{"role":"system","content":"source"},{"role":"user","content":"old"},{"role":"assistant","content":"answer"}]"#).unwrap();
    let prepared = prepare_for_role(Some(&originals), true);
    assert_eq!(prepared.target_indices, Some(vec![1]));
    let updated: Vec<Message> =
        serde_json::from_str(r#"[{"role":"user","content":"MASK"}]"#).unwrap();
    let merged = reconcile::messages(
        prepared.original_messages.as_deref(),
        &updated,
        prepared.target_indices.as_deref(),
    );
    assert_eq!(
        serde_json::to_string(&merged).unwrap(),
        r#"[{"role":"system","content":"source"},{"role":"user","content":"MASK"},{"role":"assistant","content":"answer"}]"#
    );
    assert_eq!(
        reconcile::messages(Some(&originals), &updated, Some(&[])).len(),
        1
    );
    assert_eq!(reconcile::messages(None, &updated, Some(&[7])).len(), 1);
    assert!(prepare_for_role(None, true).payload_messages.is_none());
    assert!(
        prepare_for_role(Some(&originals[..1]), true)
            .payload_messages
            .is_none()
    );
    assert_eq!(
        prepare_for_role(Some(&originals), false)
            .payload_messages
            .unwrap()
            .len(),
        3
    );
}

#[test]
fn replays_the_bedrock_messages_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/bedrock_messages");
    let paths: Vec<_> = fs::read_dir(corpus).unwrap().collect();
    assert!(!paths.is_empty());
    assert!(paths.into_iter().all(|entry| {
        let bytes = fs::read(entry.unwrap().path()).unwrap();
        let message = serde_json::from_slice::<Message>(&bytes);
        let response = serde_json::from_slice::<ModelResponse>(&bytes);
        let selection = serde_json::from_slice::<SelectionContext>(&bytes);
        message.is_ok() || response.is_ok() || selection.is_ok()
    }));
}
