#![cfg(test)]

use rstest::rstest;

use crate::roi_cache::fallback::unavailable_record;
use crate::roi_github::error::SourceError;
use crate::roi_github::models::GitHubUser;

use super::support::{Routes, github, pull_item};

#[rstest]
#[case::no_user(None)]
#[case::no_login(Some(GitHubUser { login: None }))]
#[case::empty_login(Some(GitHubUser { login: Some(String::new()) }))]
#[tokio::test]
async fn a_missing_or_empty_login_becomes_deleted_user(#[case] user: Option<GitHubUser>) {
    let routes = Routes::default().with(
        "/users/deleted-user",
        200,
        r#"{"email":"Ghost@Example.com"}"#,
    );
    let github = github(routes.clone());
    let mut pull = pull_item(7, None, Some("abc123"));
    pull.user = user;
    let record = unavailable_record(&github, "o/r", &pull, &SourceError::PaginationLimit).await;
    assert_eq!(record.login, "deleted-user");
    assert_eq!(record.profile_email, "ghost@example.com");
    assert_eq!(routes.requested(), vec!["/users/deleted-user".to_owned()]);
}

#[tokio::test]
async fn a_present_login_is_kept_and_looked_up() {
    let routes = Routes::default().with("/users/dev", 200, r#"{"email":"dev@example.com"}"#);
    let github = github(routes.clone());
    let pull = pull_item(7, Some("dev"), Some("abc123"));
    let record = unavailable_record(&github, "o/r", &pull, &SourceError::PaginationLimit).await;
    assert_eq!(record.login, "dev");
    assert_eq!(record.profile_email, "dev@example.com");
    assert_eq!(routes.requested(), vec!["/users/dev".to_owned()]);
}
