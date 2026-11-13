#![cfg(test)]

use super::adapters::RepositoryValue;
use super::repositories;
use super::support::{NEXT_LINK, Script, github, json_reply, with_link};

#[tokio::test]
async fn the_user_repositories_are_listed_sorted_by_update() {
    let script = Script::new(vec![json_reply(
        200,
        r#"[{"full_name":"octo/one","visibility":"private","private":true,"archived":false},
            {"full_name":"octo/two","visibility":null,"private":false,"archived":true}]"#,
    )]);
    let github = github(script.clone());
    let (repositories, has_more) = repositories::repositories(&github, "", 1).await.unwrap();
    assert_eq!(
        repositories,
        vec![
            RepositoryValue {
                full_name: "octo/one".to_owned(),
                visibility: "private".to_owned(),
                archived: false,
            },
            RepositoryValue {
                full_name: "octo/two".to_owned(),
                visibility: "public".to_owned(),
                archived: true,
            },
        ]
    );
    assert!(!has_more);
    let seen = script.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].url, "https://api.github.com/user/repos");
    assert_eq!(
        seen[0].query,
        vec![
            ("sort".to_owned(), "updated".to_owned()),
            ("direction".to_owned(), "desc".to_owned()),
            (
                "affiliation".to_owned(),
                "owner,collaborator,organization_member".to_owned()
            ),
            ("per_page".to_owned(), "100".to_owned()),
            ("page".to_owned(), "1".to_owned()),
        ]
    );
}

#[tokio::test]
async fn a_next_link_and_page_number_are_passed_through() {
    let script = Script::new(vec![with_link(
        json_reply(200, r#"[{"full_name":"octo/one"}]"#),
        NEXT_LINK,
    )]);
    let github = github(script.clone());
    let (_, has_more) = repositories::repositories(&github, "", 3).await.unwrap();
    assert!(has_more);
    assert_eq!(
        script.seen()[0].query.last().unwrap(),
        &("page".to_owned(), "3".to_owned())
    );
}
