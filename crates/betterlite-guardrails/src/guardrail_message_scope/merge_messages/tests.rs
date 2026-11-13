#![cfg(test)]

use rstest::rstest;

use super::merge_scoped_messages;

#[rstest]
#[case::same_count_replaces_in_place(vec!["s", "u1", "a", "u2"], vec![1, 3], vec!["U1", "U2"], vec!["s", "U1", "a", "U2"])]
#[case::fewer_returned_drops_the_trailing_scoped_ones(vec!["s", "u1", "a", "u2"], vec![1, 3], vec!["U1"], vec!["s", "U1", "a"])]
#[case::none_returned_drops_every_scoped_one(vec!["s", "u1", "a", "u2"], vec![1, 3], vec![], vec!["s", "a"])]
#[case::more_returned_follow_the_last_scoped_one(vec!["s", "u1", "a", "u2", "tail"], vec![1, 3], vec!["U1", "U2", "X", "Y"], vec!["s", "U1", "a", "U2", "X", "Y", "tail"])]
#[case::extra_after_last_scoped_at_the_end(vec!["u1"], vec![0], vec!["U1", "X"], vec!["U1", "X"])]
#[case::empty_scope_keeps_everything(vec!["s", "a"], vec![], vec![], vec!["s", "a"])]
#[case::empty_scope_ignores_returned_messages(vec!["s", "a"], vec![], vec!["X"], vec!["s", "a"])]
#[case::unscoped_messages_stay_put(vec!["s", "u", "a"], vec![1], vec!["U"], vec!["s", "U", "a"])]
#[case::dropped_middle_scoped_message(vec!["u1", "a", "u2", "b", "u3"], vec![0, 2, 4], vec!["U1", "U2"], vec!["U1", "a", "U2", "b"])]
#[case::one_returned_for_two_scoped(vec!["u1", "u2"], vec![0, 1], vec!["U"], vec!["U"])]
#[case::empty_conversation(vec![], vec![], vec![], vec![])]
fn the_guardrailed_messages_replace_the_scoped_ones(
    #[case] full: Vec<&str>,
    #[case] scoped_indices: Vec<usize>,
    #[case] guardrailed: Vec<&str>,
    #[case] expected: Vec<&str>,
) {
    assert_eq!(
        merge_scoped_messages(&full, &scoped_indices, &guardrailed),
        expected
    );
}

#[test]
fn an_index_past_the_conversation_is_ignored() {
    let merged = merge_scoped_messages(&["a", "b"], &[0, 5], &["A", "B"]);
    assert_eq!(merged, ["A", "b"]);
}
