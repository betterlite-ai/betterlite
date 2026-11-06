#![cfg(test)]

use super::adapters::RepositoryValue;
use super::support::{NEXT_LINK, Script, github, json_reply, with_link};

#[tokio::test]
async fn the_query_matches_a_case_insensitive_substring_of_the_full_name() {
    let script = Script::new(vec![
        with_link(
            json_reply(
                200,
                r#"[{"full_name":"alpha/API"},{"full_name":"beta/other"}]"#,
            ),
            NEXT_LINK,
        ),
        json_reply(
            200,
            r#"[{"full_name":"gamma/api-client"},{"full_name":"delta/web"}]"#,
        ),
    ]);
    let github = github(script.clone());
    let (repositories, has_more) = github.repositories("Api", 1).await.unwrap();
    assert_eq!(
        repositories,
        vec![
            RepositoryValue {
                full_name: "alpha/API".to_owned(),
                visibility: "public".to_owned(),
                archived: false,
            },
            RepositoryValue {
                full_name: "gamma/api-client".to_owned(),
                visibility: "public".to_owned(),
                archived: false,
            },
        ]
    );
    assert!(!has_more);
    let seen = script.seen();
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[0].url, "https://api.github.com/user/repos");
    assert_eq!(
        seen.iter()
            .map(|entry| entry.query.last().unwrap().1.as_str())
            .collect::<Vec<_>>(),
        vec!["1", "2"]
    );
}

#[tokio::test]
async fn a_query_without_matches_returns_nothing() {
    let script = Script::new(vec![json_reply(200, r#"[{"full_name":"alpha/API"}]"#)]);
    let github = github(script.clone());
    let (repositories, has_more) = github.repositories("missing", 1).await.unwrap();
    assert!(repositories.is_empty());
    assert!(!has_more);
}

#[tokio::test]
async fn the_search_starts_at_the_page_the_caller_asked_for() {
    let script = Script::new(vec![json_reply(200, r#"[{"full_name":"alpha/API"}]"#)]);
    let github = github(script.clone());
    github.repositories("api", 2).await.unwrap();
    assert_eq!(
        script.seen()[0].query.last().unwrap(),
        &("page".to_owned(), "11".to_owned())
    );
}
