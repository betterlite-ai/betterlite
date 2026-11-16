use betterlite_models::decisions::Json;
use rstest::rstest;

use super::support::json;
use crate::logging_streams::served_texts::{
    MAX_SERVED_CHOICES, MAX_SERVED_TEXT_BYTES, served_output_texts, served_stream_output_texts,
};

fn text(value: &str) -> Option<String> {
    Some(value.to_owned())
}

fn chunk(index: &str, content: &str) -> Json {
    json(&format!(
        r#"{{"choices":[{{"index":{index},"delta":{{"content":"{content}"}}}}]}}"#
    ))
}

fn chunks(entries: &[(String, String)]) -> Vec<Json> {
    entries
        .iter()
        .map(|(index, content)| chunk(index, content))
        .collect()
}

fn joins_chat_choice_texts() {
    let response = json(r#"{"choices":[{"message":{"content":"a"}},{"message":{"content":"b"}}]}"#);
    assert_eq!(
        served_output_texts(&response),
        Some(vec![text("a"), text("b")])
    );
}

fn keeps_a_missing_message_as_a_hole() {
    let response = json(r#"{"choices":[{"message":null},{"message":{"content":"b"}}]}"#);
    assert_eq!(served_output_texts(&response), Some(vec![None, text("b")]));
}

fn drops_chat_choices_without_any_text() {
    let response = json(r#"{"choices":[{"message":{}},{"message":{"content":null}}]}"#);
    assert_eq!(served_output_texts(&response), None);
}

fn drops_a_response_without_choices() {
    assert_eq!(served_output_texts(&json(r#"{"id":"x"}"#)), None);
}

fn drops_a_response_that_is_not_an_object() {
    assert_eq!(served_output_texts(&Json::Text("x".to_owned())), None);
}

fn drops_choices_that_are_not_a_list() {
    assert_eq!(served_output_texts(&json(r#"{"choices":{}}"#)), None);
    assert_eq!(served_output_texts(&json(r#"{"choices":null}"#)), None);
}

fn drops_a_choice_that_is_not_an_object() {
    assert_eq!(served_output_texts(&json(r#"{"choices":["a"]}"#)), None);
}

fn drops_a_message_that_is_not_an_object() {
    assert_eq!(
        served_output_texts(&json(r#"{"choices":[{"message":"a"}]}"#)),
        None
    );
}

fn accepts_every_message_role() {
    let response = json(
        r#"{"choices":[{"message":{"role":"assistant","content":"a"}},{"message":{"role":"user","content":"b"}},{"message":{"role":"system","content":"c"}},{"message":{"role":"tool","content":"d"}},{"message":{"role":"function","content":"e"}}]}"#,
    );
    assert_eq!(
        served_output_texts(&response),
        Some(vec![text("a"), text("b"), text("c"), text("d"), text("e"),])
    );
}

fn drops_a_role_outside_the_message_roles() {
    assert_eq!(
        served_output_texts(&json(
            r#"{"choices":[{"message":{"role":"critic","content":"a"}}]}"#
        )),
        None
    );
}

fn drops_a_role_that_is_not_a_name() {
    assert_eq!(
        served_output_texts(&json(
            r#"{"choices":[{"message":{"role":7,"content":"a"}}]}"#
        )),
        None
    );
}

fn drops_content_that_is_not_text() {
    assert_eq!(
        served_output_texts(&json(r#"{"choices":[{"message":{"content":["a"]}}]}"#)),
        None
    );
    assert_eq!(
        served_output_texts(&json(r#"{"choices":[{"message":{"content":true}}]}"#)),
        None
    );
}

fn keeps_empty_chat_content() {
    assert_eq!(
        served_output_texts(&json(r#"{"choices":[{"message":{"content":""}}]}"#)),
        Some(vec![text("")])
    );
}

fn joins_stream_text_pieces() {
    let pieces = chunks(&[
        ("0".to_owned(), "he".to_owned()),
        ("0".to_owned(), "llo".to_owned()),
    ]);
    assert_eq!(
        served_stream_output_texts(&pieces),
        Some(vec![text("hello")])
    );
}

fn keeps_holes_for_sparse_stream_indices() {
    let pieces = chunks(&[
        ("0".to_owned(), "a".to_owned()),
        ("2".to_owned(), "b".to_owned()),
    ]);
    assert_eq!(
        served_stream_output_texts(&pieces),
        Some(vec![text("a"), None, text("b")])
    );
}

fn treats_a_missing_stream_index_as_zero() {
    let pieces = vec![json(r#"{"choices":[{"delta":{"content":"a"}}]}"#)];
    assert_eq!(served_stream_output_texts(&pieces), Some(vec![text("a")]));
}

fn treats_a_null_stream_index_as_zero() {
    let pieces = vec![json(
        r#"{"choices":[{"index":null,"delta":{"content":"a"}}]}"#,
    )];
    assert_eq!(served_stream_output_texts(&pieces), Some(vec![text("a")]));
}

fn ignores_negative_stream_indices() {
    let mixed = chunks(&[
        ("-1".to_owned(), "a".to_owned()),
        ("0".to_owned(), "b".to_owned()),
    ]);
    assert_eq!(served_stream_output_texts(&mixed), Some(vec![text("b")]));
    let negative_only = chunks(&[("-1".to_owned(), "a".to_owned())]);
    assert_eq!(served_stream_output_texts(&negative_only), None);
}

fn ignores_a_stream_index_that_is_not_a_number() {
    let pieces = chunks(&[
        ("\"0\"".to_owned(), "a".to_owned()),
        ("0".to_owned(), "b".to_owned()),
    ]);
    assert_eq!(served_stream_output_texts(&pieces), Some(vec![text("b")]));
}

fn drops_a_stream_index_past_the_choice_bound() {
    let past = MAX_SERVED_CHOICES.to_string();
    let pieces = chunks(&[(past, "a".to_owned())]);
    assert_eq!(served_stream_output_texts(&pieces), None);
}

fn keeps_the_last_stream_index_inside_the_choice_bound() {
    let inside = (MAX_SERVED_CHOICES - 1).to_string();
    let pieces = chunks(&[(inside, "a".to_owned())]);
    let texts = served_stream_output_texts(&pieces).unwrap();
    assert_eq!(texts.len(), MAX_SERVED_CHOICES);
    assert_eq!(texts.last(), Some(&text("a")));
}

fn drops_a_stream_piece_past_the_text_bound() {
    let oversized = "x".repeat(MAX_SERVED_TEXT_BYTES + 1);
    let pieces = chunks(&[("0".to_owned(), oversized)]);
    assert_eq!(served_stream_output_texts(&pieces), None);
}

fn keeps_stream_pieces_that_fill_the_text_bound() {
    let filling = "x".repeat(MAX_SERVED_TEXT_BYTES);
    let pieces = chunks(&[("0".to_owned(), filling), ("0".to_owned(), "y".to_owned())]);
    let texts = served_stream_output_texts(&pieces).unwrap();
    assert_eq!(
        texts[0].as_ref().map(String::len),
        Some(MAX_SERVED_TEXT_BYTES)
    );
}

fn keeps_a_stream_piece_after_a_dropped_one() {
    let oversized = "x".repeat(MAX_SERVED_TEXT_BYTES + 1);
    let pieces = chunks(&[
        ("0".to_owned(), oversized),
        ("0".to_owned(), "y".to_owned()),
    ]);
    assert_eq!(served_stream_output_texts(&pieces), Some(vec![text("y")]));
}

fn skips_stream_chunks_that_are_not_objects() {
    let pieces = vec![Json::Text("x".to_owned()), chunk("0", "a")];
    assert_eq!(served_stream_output_texts(&pieces), Some(vec![text("a")]));
}

fn skips_stream_chunks_without_choices() {
    let pieces = vec![json(r#"{"id":"x"}"#), chunk("0", "a")];
    assert_eq!(served_stream_output_texts(&pieces), Some(vec![text("a")]));
}

fn skips_stream_choices_that_are_not_objects() {
    let pieces = vec![json(r#"{"choices":["a"]}"#), chunk("0", "b")];
    assert_eq!(served_stream_output_texts(&pieces), Some(vec![text("b")]));
}

fn skips_a_stream_delta_that_is_not_an_object() {
    let pieces = vec![
        json(r#"{"choices":[{"delta":"a"}]}"#),
        json(r#"{"choices":[{"delta":null}]}"#),
        chunk("0", "b"),
    ];
    assert_eq!(served_stream_output_texts(&pieces), Some(vec![text("b")]));
}

fn skips_stream_content_that_is_not_text() {
    let pieces = vec![
        json(r#"{"choices":[{"delta":{"content":true}}]}"#),
        json(r#"{"choices":[{"delta":{"content":7}}]}"#),
        json(r#"{"choices":[{"delta":{"content":["a"]}}]}"#),
        json(r#"{"choices":[{"delta":{"content":null}}]}"#),
    ];
    assert_eq!(served_stream_output_texts(&pieces), None);
}

fn keeps_empty_stream_text() {
    let pieces = vec![chunk("0", "")];
    assert_eq!(served_stream_output_texts(&pieces), Some(vec![text("")]));
}

fn ignores_tool_call_stream_deltas() {
    let pieces = vec![json(
        r#"{"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call-1","type":"function","function":{"name":"lookup","arguments":""}}]}}]}"#,
    )];
    assert_eq!(served_stream_output_texts(&pieces), None);
}

fn counts_a_stream_index_without_text_as_a_hole() {
    let pieces = vec![
        json(r#"{"choices":[{"index":1,"delta":{}}]}"#),
        chunk("0", "a"),
    ];
    assert_eq!(
        served_stream_output_texts(&pieces),
        Some(vec![text("a"), None])
    );
}

fn drops_an_empty_stream() {
    assert_eq!(served_stream_output_texts(&[]), None);
}

fn joins_stream_text_across_higher_indices() {
    let pieces = chunks(&[
        ("1".to_owned(), "a".to_owned()),
        ("0".to_owned(), "b".to_owned()),
    ]);
    assert_eq!(
        served_stream_output_texts(&pieces),
        Some(vec![text("b"), text("a")])
    );
}

#[rstest]
#[case::served_output_texts_joins_chat_choice_texts(joins_chat_choice_texts)]
#[case::served_output_texts_keeps_a_missing_message_as_a_hole(keeps_a_missing_message_as_a_hole)]
#[case::served_output_texts_drops_choices_without_any_text(drops_chat_choices_without_any_text)]
#[case::served_output_texts_drops_a_response_without_choices(drops_a_response_without_choices)]
#[case::served_output_texts_drops_a_response_that_is_not_an_object(
    drops_a_response_that_is_not_an_object
)]
#[case::served_output_texts_drops_choices_that_are_not_a_list(drops_choices_that_are_not_a_list)]
#[case::served_output_texts_drops_a_choice_that_is_not_an_object(
    drops_a_choice_that_is_not_an_object
)]
#[case::served_output_texts_drops_a_message_that_is_not_an_object(
    drops_a_message_that_is_not_an_object
)]
#[case::served_output_texts_accepts_every_message_role(accepts_every_message_role)]
#[case::served_output_texts_drops_a_role_outside_the_message_roles(
    drops_a_role_outside_the_message_roles
)]
#[case::served_output_texts_drops_a_role_that_is_not_a_name(drops_a_role_that_is_not_a_name)]
#[case::served_output_texts_drops_content_that_is_not_text(drops_content_that_is_not_text)]
#[case::served_output_texts_keeps_empty_chat_content(keeps_empty_chat_content)]
#[case::served_stream_output_texts_joins_stream_text_pieces(joins_stream_text_pieces)]
#[case::served_stream_output_texts_keeps_holes_for_sparse_indices(
    keeps_holes_for_sparse_stream_indices
)]
#[case::served_stream_output_texts_treats_a_missing_index_as_zero(
    treats_a_missing_stream_index_as_zero
)]
#[case::served_stream_output_texts_treats_a_null_index_as_zero(treats_a_null_stream_index_as_zero)]
#[case::served_stream_output_texts_ignores_negative_indices(ignores_negative_stream_indices)]
#[case::served_stream_output_texts_ignores_a_non_numeric_index(
    ignores_a_stream_index_that_is_not_a_number
)]
#[case::served_stream_output_texts_drops_an_index_past_the_bound(
    drops_a_stream_index_past_the_choice_bound
)]
#[case::served_stream_output_texts_keeps_the_last_index_inside_the_bound(
    keeps_the_last_stream_index_inside_the_choice_bound
)]
#[case::served_stream_output_texts_drops_a_piece_past_the_text_bound(
    drops_a_stream_piece_past_the_text_bound
)]
#[case::served_stream_output_texts_keeps_pieces_that_fill_the_text_bound(
    keeps_stream_pieces_that_fill_the_text_bound
)]
#[case::served_stream_output_texts_keeps_a_piece_after_a_dropped_one(
    keeps_a_stream_piece_after_a_dropped_one
)]
#[case::served_stream_output_texts_skips_chunks_that_are_not_objects(
    skips_stream_chunks_that_are_not_objects
)]
#[case::served_stream_output_texts_skips_chunks_without_choices(
    skips_stream_chunks_without_choices
)]
#[case::served_stream_output_texts_skips_choices_that_are_not_objects(
    skips_stream_choices_that_are_not_objects
)]
#[case::served_stream_output_texts_skips_a_delta_that_is_not_an_object(
    skips_a_stream_delta_that_is_not_an_object
)]
#[case::served_stream_output_texts_skips_content_that_is_not_text(
    skips_stream_content_that_is_not_text
)]
#[case::served_stream_output_texts_keeps_empty_text(keeps_empty_stream_text)]
#[case::served_stream_output_texts_ignores_tool_call_deltas(ignores_tool_call_stream_deltas)]
#[case::served_stream_output_texts_counts_an_index_without_text_as_a_hole(
    counts_a_stream_index_without_text_as_a_hole
)]
#[case::served_stream_output_texts_drops_an_empty_stream(drops_an_empty_stream)]
#[case::served_stream_output_texts_joins_across_higher_indices(
    joins_stream_text_across_higher_indices
)]
fn logging_streams_behavior_tests_behavior(#[case] behavior: fn()) {
    behavior();
}
