#![cfg(test)]

use std::iter::repeat_with;

use rstest::rstest;

use super::pulls;
use super::tests::{NEXT_LINK, Script, date, github, json_array, json_reply, pull_item, with_link};

#[tokio::test]
async fn pulls_select_merged_pulls_inside_the_inclusive_range() {
    let body = json_array(&[
        pull_item(1, Some("2026-01-01T00:00:00Z"), "2026-01-10T00:00:00Z"),
        pull_item(2, Some("2026-01-31T23:59:59Z"), "2026-01-10T00:00:00Z"),
        pull_item(3, Some("2025-12-31T23:59:59Z"), "2026-01-10T00:00:00Z"),
        pull_item(4, Some("2026-02-01T00:00:00Z"), "2026-01-10T00:00:00Z"),
        pull_item(5, None, "2026-01-10T00:00:00Z"),
    ]);
    let script = Script::new(vec![json_reply(200, &body)]);
    let github = github(script);
    let found = pulls(&github, "o/r", date("2026-01-01"), date("2026-01-31"))
        .await
        .unwrap();
    assert_eq!(
        found.iter().map(|pull| pull.number).collect::<Vec<_>>(),
        vec![1, 2]
    );
}

#[tokio::test]
async fn pulls_keep_the_order_github_returns_them_in() {
    let body = json_array(&[
        pull_item(3, Some("2026-01-02T00:00:00Z"), "2026-01-10T00:00:00Z"),
        pull_item(1, Some("2026-01-03T00:00:00Z"), "2026-01-10T00:00:00Z"),
        pull_item(2, Some("2026-01-04T00:00:00Z"), "2026-01-10T00:00:00Z"),
    ]);
    let script = Script::new(vec![json_reply(200, &body)]);
    let github = github(script);
    let found = pulls(&github, "o/r", date("2026-01-01"), date("2026-01-31"))
        .await
        .unwrap();
    assert_eq!(
        found.iter().map(|pull| pull.number).collect::<Vec<_>>(),
        vec![3, 1, 2]
    );
}

#[tokio::test]
async fn pulls_ask_for_closed_pulls_sorted_by_update_descending() {
    let script = Script::new(vec![json_reply(200, "[]")]);
    let github = github(script.clone());
    pulls(&github, "o/r", date("2026-01-01"), date("2026-01-31"))
        .await
        .unwrap();
    let seen = script.seen();
    assert_eq!(seen[0].method, "GET");
    assert_eq!(seen[0].url, "https://api.github.com/repos/o/r/pulls");
    assert_eq!(
        seen[0].query,
        vec![
            ("state".to_owned(), "closed".to_owned()),
            ("sort".to_owned(), "updated".to_owned()),
            ("direction".to_owned(), "desc".to_owned()),
            ("per_page".to_owned(), "100".to_owned()),
            ("page".to_owned(), "1".to_owned()),
        ]
    );
}

#[tokio::test]
async fn pulls_stop_after_a_page_whose_last_update_is_before_the_start() {
    let first = json_array(&[
        pull_item(1, Some("2026-01-05T00:00:00Z"), "2026-01-05T00:00:00Z"),
        pull_item(2, None, "2025-12-01T00:00:00Z"),
    ]);
    let script = Script::new(vec![
        with_link(json_reply(200, &first), NEXT_LINK),
        json_reply(200, "[]"),
    ]);
    let github = github(script.clone());
    let found = pulls(&github, "o/r", date("2026-01-01"), date("2026-01-31"))
        .await
        .unwrap();
    assert_eq!(
        found.iter().map(|pull| pull.number).collect::<Vec<_>>(),
        vec![1]
    );
    assert_eq!(script.seen().len(), 1);
}

#[tokio::test]
async fn pulls_judge_the_early_stop_by_the_page_last_item() {
    let first = json_array(&[
        pull_item(1, None, "2025-12-01T00:00:00Z"),
        pull_item(2, None, "2026-01-05T00:00:00Z"),
    ]);
    let script = Script::new(vec![
        with_link(json_reply(200, &first), NEXT_LINK),
        json_reply(200, "[]"),
    ]);
    let github = github(script.clone());
    pulls(&github, "o/r", date("2026-01-01"), date("2026-01-31"))
        .await
        .unwrap();
    assert_eq!(script.seen().len(), 2);
}

#[rstest]
#[case::the_start_date_itself_continues("2026-01-01T00:00:00Z", 2)]
#[case::a_later_update_continues("2026-01-02T00:00:00Z", 2)]
#[case::an_earlier_update_stops("2025-12-31T23:59:59Z", 1)]
#[tokio::test]
async fn pulls_compare_the_last_updated_date_with_the_start_day(
    #[case] updated_at: &str,
    #[case] expected_requests: usize,
) {
    let first = json_array(&[pull_item(1, None, updated_at)]);
    let script = Script::new(vec![
        with_link(json_reply(200, &first), NEXT_LINK),
        json_reply(200, "[]"),
    ]);
    let github = github(script.clone());
    pulls(&github, "o/r", date("2026-01-01"), date("2026-01-31"))
        .await
        .unwrap();
    assert_eq!(script.seen().len(), expected_requests);
}

#[tokio::test]
async fn pulls_stop_at_ten_thousand_pages() {
    let page = json_array(&[pull_item(
        1,
        Some("2026-01-05T00:00:00Z"),
        "2026-01-05T00:00:00Z",
    )]);
    let script = Script::new(
        repeat_with(|| with_link(json_reply(200, &page), NEXT_LINK))
            .take(10_000)
            .collect(),
    );
    let github = github(script.clone());
    let error = pulls(&github, "o/r", date("2026-01-01"), date("2026-01-31"))
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "GitHub's pagination limit was reached. Narrow the date range."
    );
    assert_eq!(script.seen().len(), 10_000);
}

#[tokio::test]
async fn pulls_page_past_an_empty_page_the_early_stop_cannot_judge() {
    let second = json_array(&[pull_item(
        1,
        Some("2026-01-05T00:00:00Z"),
        "2026-01-05T00:00:00Z",
    )]);
    let script = Script::new(vec![
        with_link(json_reply(200, "[]"), NEXT_LINK),
        json_reply(200, &second),
    ]);
    let github = github(script.clone());
    let found = pulls(&github, "o/r", date("2026-01-01"), date("2026-01-31"))
        .await
        .unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(script.seen().len(), 2);
}
