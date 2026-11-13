#![cfg(test)]

use futures_util::TryStreamExt as _;
use rstest::rstest;
use std::iter::repeat_with;

use super::models::RepositoryItem;
use super::pages;
use super::support::{NEXT_LINK, Script, github, json_reply, with_link};

const LIMIT_ERROR: &str = "GitHub's pagination limit was reached. Narrow the date range.";

fn endless(limit: usize) -> Script {
    Script::new(
        repeat_with(|| with_link(json_reply(200, "[]"), NEXT_LINK))
            .take(limit)
            .collect(),
    )
}

#[rstest]
#[case::no_page_is_allowed(0, 0)]
#[case::the_limit_counts_fetched_pages(3, 3)]
#[case::ten_pages_is_the_default_search_depth(10, 10)]
#[tokio::test]
async fn a_page_marked_next_after_the_last_allowed_page_raises(
    #[case] limit: u32,
    #[case] expected_requests: usize,
) {
    let script = endless(usize::try_from(limit).unwrap_or(0));
    let github = github(script.clone());
    let collected = pages::pages::<RepositoryItem, _>(&github, "user/repos", &[], limit)
        .try_collect::<Vec<Vec<RepositoryItem>>>()
        .await;
    assert_eq!(collected.unwrap_err().to_string(), LIMIT_ERROR);
    assert_eq!(script.seen().len(), expected_requests);
}

#[tokio::test]
async fn the_last_fetched_page_is_the_one_that_raises() {
    let script = endless(3);
    let github = github(script.clone());
    let collected = pages::pages::<RepositoryItem, _>(&github, "user/repos", &[], 3)
        .try_collect::<Vec<Vec<RepositoryItem>>>()
        .await;
    assert_eq!(collected.unwrap_err().to_string(), LIMIT_ERROR);
    let seen = script.seen();
    assert_eq!(
        seen.last().unwrap().query.last().unwrap(),
        &("page".to_owned(), "3".to_owned())
    );
}

#[tokio::test]
async fn the_stream_can_be_read_page_by_page() {
    let script = endless(1);
    let github = github(script.clone());
    let params = [("state".to_owned(), "closed".to_owned())];
    let mut stream = Box::pin(pages::pages::<RepositoryItem, _>(
        &github,
        "user/repos",
        &params,
        1,
    ));
    let first = stream.try_next().await.unwrap();
    assert_eq!(first, Some(Vec::new()));
    let error = stream.try_next().await.unwrap_err();
    assert_eq!(error.to_string(), LIMIT_ERROR);
    assert_eq!(
        script.seen()[0].query,
        vec![
            ("state".to_owned(), "closed".to_owned()),
            ("per_page".to_owned(), "100".to_owned()),
            ("page".to_owned(), "1".to_owned()),
        ]
    );
}
