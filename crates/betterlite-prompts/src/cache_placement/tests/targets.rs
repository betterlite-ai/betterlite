use rstest::rstest;

use crate::arguments::json::Object;
use crate::cache_placement::targets::resolve_target_indices;
use crate::cache_placement::tests::support::{capture_warnings, messages, point as single_point};
use crate::message_content::{Content, Message};

const CONVERSATION: &str = r#"[
    {"role": "user", "content": "one"},
    {"role": "assistant", "content": "two"},
    {"role": "user", "content": "three"}
]"#;

fn messages_of() -> Vec<Message> {
    messages(CONVERSATION)
}

fn twelve_users() -> Vec<Message> {
    (0..12_usize)
        .map(|index| Message {
            role: "user".to_owned(),
            content: Some(Content::Text(index.to_string())),
            name: None,
            provider_specific_fields: Object::new(),
        })
        .collect()
}

#[rstest]
#[case::first(0, vec![0])]
#[case::middle(1, vec![1])]
#[case::last(2, vec![2])]
#[case::negative_last(-1, vec![2])]
#[case::negative_middle(-2, vec![1])]
#[case::negative_first(-3, vec![0])]
#[case::negative_past_start(-4, vec![])]
#[case::far_negative(-100, vec![])]
#[case::at_length(3, vec![])]
#[case::far_positive(99, vec![])]
fn an_integer_index_targets_one_message(#[case] index: i64, #[case] expected: Vec<usize>) {
    let point = single_point(&format!(r#"{{"index": {index}}}"#));
    assert_eq!(resolve_target_indices(&point, &messages_of()), expected);
}

#[rstest]
#[case::number("1", vec![1])]
#[case::negative("-1", vec![2])]
#[case::negative_past_start("-100", vec![])]
#[case::whitespace(" 1 ", vec![1])]
#[case::plus_sign("+2", vec![2])]
#[case::out_of_bounds("3", vec![])]
#[case::underscore_pair("1_0", vec![])]
#[case::huge("99999999999999999999", vec![])]
fn a_text_index_is_read_as_an_integer(#[case] index: &str, #[case] expected: Vec<usize>) {
    let point = single_point(&format!(r#"{{"index": "{index}"}}"#));
    assert_eq!(resolve_target_indices(&point, &messages_of()), expected);
}

#[rstest]
#[case::underscore_pair("1_0", vec![10])]
#[case::negative_underscore_pair("-1_0", vec![2])]
#[case::plus_underscore_pair("+1_0", vec![10])]
#[case::huge_positive("99999999999999999999", vec![])]
#[case::huge_negative("-99999999999999999999", vec![])]
#[case::double_underscore("1__0", vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11])]
#[case::leading_underscore("_1", vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11])]
#[case::trailing_underscore("1_", vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11])]
#[case::empty("", vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11])]
#[case::not_a_number("first", vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11])]
#[case::fullwidth("\u{ff11}", vec![1])]
#[case::arabic_indic_underscore("\u{661}_\u{660}", vec![10])]
#[case::devanagari("\u{967}", vec![1])]
fn a_text_index_is_read_like_python_int(#[case] index: &str, #[case] expected: Vec<usize>) {
    let point = single_point(&format!(r#"{{"index": "{index}", "role": "user"}}"#));
    assert_eq!(resolve_target_indices(&point, &twelve_users()), expected);
}

#[rstest]
#[case::user(r#"{"role": "user"}"#, vec![0, 2])]
#[case::assistant(r#"{"role": "assistant"}"#, vec![1])]
#[case::system(r#"{"role": "system"}"#, vec![])]
#[case::nobody("{}", vec![])]
fn a_role_targets_every_matching_message(#[case] wire: &str, #[case] expected: Vec<usize>) {
    let point = single_point(wire);
    assert_eq!(resolve_target_indices(&point, &messages_of()), expected);
}

#[test]
fn an_index_outranks_a_role_that_points_elsewhere() {
    let point = single_point(r#"{"index": 0, "role": "assistant"}"#);
    assert_eq!(resolve_target_indices(&point, &messages_of()), vec![0]);
}

#[test]
fn an_out_of_bounds_index_does_not_fall_back_to_the_role() {
    let point = single_point(r#"{"index": 9, "role": "user"}"#);
    assert_eq!(
        resolve_target_indices(&point, &messages_of()),
        Vec::<usize>::new()
    );
}

#[test]
fn a_text_index_that_is_not_a_number_targets_by_role() {
    let point = single_point(r#"{"index": "1st", "role": "assistant"}"#);
    assert_eq!(resolve_target_indices(&point, &messages_of()), vec![1]);
}

#[test]
fn a_text_index_that_is_not_a_number_without_a_role_targets_nothing() {
    let point = single_point(r#"{"index": "1st"}"#);
    assert_eq!(
        resolve_target_indices(&point, &messages_of()),
        Vec::<usize>::new()
    );
}

#[test]
fn an_out_of_bounds_index_logs_one_warning() {
    let (events, _captured) = capture_warnings();
    let point = single_point(r#"{"index": -100}"#);
    assert!(resolve_target_indices(&point, &messages_of()).is_empty());
    let warnings = events.lock().unwrap().clone();
    assert_eq!(warnings.len(), 1);
}
