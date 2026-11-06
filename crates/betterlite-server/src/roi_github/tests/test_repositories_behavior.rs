#![cfg(test)]

use super::support::{Script, github, json_reply, reply};

#[tokio::test]
async fn read_access_is_checked_with_the_repository_and_its_closed_pulls() {
    let script = Script::new(vec![
        json_reply(200, "{}"),
        json_reply(200, "[]"),
        json_reply(200, "{}"),
        json_reply(200, "[]"),
    ]);
    let github = github(script.clone());
    github
        .test_repositories(&["Owner/Repo".to_owned(), "two/repo".to_owned()])
        .await
        .unwrap();
    let seen = script.seen();
    assert_eq!(seen.len(), 4);
    assert_eq!(
        seen.iter()
            .map(|entry| (entry.method.as_str(), entry.url.as_str()))
            .collect::<Vec<_>>(),
        vec![
            ("GET", "https://api.github.com/repos/Owner/Repo"),
            ("GET", "https://api.github.com/repos/Owner/Repo/pulls"),
            ("GET", "https://api.github.com/repos/two/repo"),
            ("GET", "https://api.github.com/repos/two/repo/pulls"),
        ]
    );
    assert_eq!(
        seen[1].query,
        vec![
            ("per_page".to_owned(), "1".to_owned()),
            ("state".to_owned(), "closed".to_owned()),
        ]
    );
    assert!(seen[0].query.is_empty());
}

#[tokio::test]
async fn an_unreadable_repository_stops_the_check() {
    let script = Script::new(vec![reply(404)]);
    let github = github(script.clone());
    let error = github
        .test_repositories(&["Owner/Repo".to_owned()])
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "GitHub repository or organization not found. Check its name, token access, and API URL. (HTTP 404)"
    );
    assert_eq!(script.seen().len(), 1);
}
