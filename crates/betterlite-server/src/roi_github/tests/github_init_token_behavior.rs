#![cfg(test)]

use axum::http::Method;
use betterlite_roi::settings::ROISettings;

use super::client::{ACCEPT_GITHUB_JSON, GitHub};
use super::support::{Script, github, github_with_token, json_reply};

#[tokio::test]
async fn a_configured_token_becomes_a_bearer_authorization_header() {
    let script = Script::new(vec![json_reply(200, "[]")]);
    let github = github_with_token(script.clone(), "ghp_secret", "https://github.example/api");
    github
        .request(Method::GET, "user/repos", &[], None)
        .await
        .unwrap();
    let seen = script.seen();
    assert_eq!(seen[0].accept.as_deref(), Some(ACCEPT_GITHUB_JSON));
    assert_eq!(seen[0].authorization.as_deref(), Some("Bearer ghp_secret"));
    assert_eq!(seen[0].url, "https://github.example/api/user/repos");
}

#[tokio::test]
async fn an_empty_token_sends_only_the_accept_header() {
    let script = Script::new(vec![json_reply(200, "[]")]);
    let github = github(script.clone());
    github
        .request(Method::GET, "user/repos", &[], None)
        .await
        .unwrap();
    let seen = script.seen();
    assert_eq!(seen[0].accept.as_deref(), Some(ACCEPT_GITHUB_JSON));
    assert_eq!(seen[0].authorization, None);
}

#[tokio::test]
async fn the_api_url_drops_trailing_slashes_and_paths_lose_leading_ones() {
    let script = Script::new(vec![json_reply(200, "[]")]);
    let github = github_with_token(script.clone(), "", "https://github.example/api/");
    github
        .request(Method::GET, "/user/repos", &[], None)
        .await
        .unwrap();
    assert_eq!(
        script.seen()[0].url,
        "https://github.example/api/user/repos"
    );
}

#[test]
fn a_token_a_header_cannot_carry_fails_when_the_client_is_built() {
    let settings: ROISettings = serde_json::from_str("{\"github_token\":\"bad\\nvalue\"}").unwrap();
    let error = GitHub::new(&settings, Script::new(Vec::new()))
        .err()
        .unwrap();
    assert_eq!(
        error.to_string(),
        "Could not reach GitHub. Check the API URL and network connection."
    );
}
