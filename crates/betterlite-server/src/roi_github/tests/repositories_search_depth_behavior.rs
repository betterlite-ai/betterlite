#![cfg(test)]

use std::iter::repeat_with;

use super::repositories;
use super::support::{NEXT_LINK, Script, github, json_reply, with_link};

fn ten_pages() -> Script {
    Script::new(
        repeat_with(|| with_link(json_reply(200, "[]"), NEXT_LINK))
            .take(10)
            .collect(),
    )
}

#[tokio::test]
async fn the_search_reads_at_most_ten_github_pages() {
    let script = ten_pages();
    let github = github(script.clone());
    let (repositories, has_more) = repositories::repositories(&github, "api", 1).await.unwrap();
    assert!(repositories.is_empty());
    assert!(has_more);
    let seen = script.seen();
    assert_eq!(seen.len(), 10);
    assert_eq!(
        seen.iter()
            .map(|entry| entry.query.last().unwrap().1.as_str())
            .collect::<Vec<_>>(),
        vec!["1", "2", "3", "4", "5", "6", "7", "8", "9", "10"]
    );
}

#[tokio::test]
async fn a_later_client_page_continues_the_github_page_numbering() {
    let script = ten_pages();
    let github = github(script.clone());
    let (_, has_more) = repositories::repositories(&github, "api", 2).await.unwrap();
    assert!(has_more);
    let seen = script.seen();
    assert_eq!(seen.len(), 10);
    assert_eq!(
        seen.iter()
            .map(|entry| entry.query.last().unwrap().1.as_str())
            .collect::<Vec<_>>(),
        vec!["11", "12", "13", "14", "15", "16", "17", "18", "19", "20"]
    );
}
