#![cfg(test)]

use futures_util::TryStreamExt as _;

use super::models::RepositoryItem;
use super::support::{NEXT_LINK, Script, github, json_reply, with_link};

#[tokio::test]
async fn pages_stop_at_the_first_answer_without_a_next_link() {
    let script = Script::new(vec![
        with_link(
            json_reply(200, r#"[{"full_name":"octo/first"}]"#),
            NEXT_LINK,
        ),
        json_reply(200, r#"[{"full_name":"octo/second"}]"#),
        json_reply(200, r#"[{"full_name":"octo/unused"}]"#),
    ]);
    let github = github(script.clone());
    let pages = github
        .pages::<RepositoryItem>("user/repos", &[], 10000)
        .try_collect::<Vec<Vec<RepositoryItem>>>()
        .await
        .unwrap();
    let names = pages
        .iter()
        .flatten()
        .map(|item| item.full_name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(names, vec!["octo/first", "octo/second"]);
    assert_eq!(script.seen().len(), 2);
}

#[tokio::test]
async fn a_single_page_without_a_next_link_is_collected_once() {
    let script = Script::new(vec![json_reply(200, r#"[{"full_name":"octo/only"}]"#)]);
    let github = github(script.clone());
    let pages = github
        .pages::<RepositoryItem>("user/repos", &[], 10)
        .try_collect::<Vec<Vec<RepositoryItem>>>()
        .await
        .unwrap();
    assert_eq!(pages.len(), 1);
    assert_eq!(pages[0][0].full_name, "octo/only");
    assert_eq!(script.seen().len(), 1);
}
