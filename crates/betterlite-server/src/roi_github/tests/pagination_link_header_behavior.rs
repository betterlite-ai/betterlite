#![cfg(test)]

use rstest::rstest;

use super::models::{PAGINATION_ERROR, RepositoryItem};
use super::pages;
use super::support::{Script, github, json_reply, with_link};

const EMPTY: &str = "[]";

#[rstest]
#[case::next_link("</user/repos?page=2>; rel=\"next\"", true)]
#[case::last_link_only("</user/repos?page=2>; rel=\"last\"", false)]
#[case::no_link("", false)]
#[case::next_token_is_case_sensitive("</user/repos?page=2>; REL=\"NEXT\"", false)]
#[case::next_after_last("</a>; rel=\"last\", </b>; rel=\"next\"", true)]
#[case::relative_next("</x>; rel=next", false)]
#[tokio::test]
async fn the_link_header_decides_whether_another_page_follows(
    #[case] link: &str,
    #[case] expected: bool,
) {
    let mut reply = json_reply(200, EMPTY);
    if !link.is_empty() {
        reply = with_link(reply, link);
    }
    let script = Script::new(vec![reply]);
    let github = github(script.clone());
    let (items, has_next) =
        pages::fetch_page::<RepositoryItem, _>(&github, "user/repos", &[], 1, PAGINATION_ERROR)
            .await
            .unwrap();
    assert!(items.is_empty());
    assert_eq!(has_next, expected);
}

#[tokio::test]
async fn repeated_link_headers_are_joined_before_the_search() {
    let mut reply = with_link(json_reply(200, EMPTY), "</a>; rel=\"prev\"");
    reply = with_link(reply, "</b>; rel=\"next\"");
    let script = Script::new(vec![reply]);
    let github = github(script.clone());
    let (_, has_next) =
        pages::fetch_page::<RepositoryItem, _>(&github, "user/repos", &[], 1, PAGINATION_ERROR)
            .await
            .unwrap();
    assert!(has_next);
}
