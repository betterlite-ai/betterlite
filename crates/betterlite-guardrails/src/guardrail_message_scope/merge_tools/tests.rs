#![cfg(test)]

use rstest::rstest;

use super::merge_returned_tools;

fn name_of(tool: &&str) -> Option<String> {
    let (name, _) = tool.split_once(':').unwrap_or((tool, ""));
    if name.is_empty() {
        None
    } else {
        Some(name.to_owned())
    }
}

#[rstest]
#[case::returned_adds_new_tool(vec!["a:1"], vec!["a:2", "b:2"], vec!["a:1", "b:2"])]
#[case::request_tool_wins_over_returned(vec!["a:request"], vec!["a:returned"], vec!["a:request"])]
#[case::first_returned_tool_wins_over_later_one(vec![], vec!["a:1", "a:2"], vec!["a:1"])]
#[case::returned_order_is_kept(vec!["z:0"], vec!["c:1", "b:2"], vec!["z:0", "c:1", "b:2"])]
#[case::request_duplicates_are_kept(vec!["a:1", "a:2"], vec![], vec!["a:1", "a:2"])]
#[case::unnamed_returned_tools_are_always_added(vec![":x"], vec![":y", ":y"], vec![":x", ":y", ":y"])]
#[case::unnamed_request_tool_blocks_nothing(vec![":x"], vec!["a:1"], vec![":x", "a:1"])]
#[case::empty_returned_keeps_the_request(vec!["a:1"], vec![], vec!["a:1"])]
#[case::both_empty(vec![], vec![], vec![])]
fn returned_tools_join_the_request_tools_by_name(
    #[case] request: Vec<&str>,
    #[case] returned: Vec<&str>,
    #[case] expected: Vec<&str>,
) {
    assert_eq!(merge_returned_tools(&request, &returned, name_of), expected);
}
