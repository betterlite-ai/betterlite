#![cfg(test)]

use axum::http::Method;

use super::client::FOLLOW_REDIRECTS;
use super::support::{Script, github, json_reply, reply};

const _: () = assert!(!FOLLOW_REDIRECTS);

#[tokio::test]
async fn a_redirect_is_returned_instead_of_followed() {
    let mut redirect = reply(302);
    redirect.headers.push((
        "location".to_owned(),
        "https://elsewhere.example/next".to_owned(),
    ));
    let script = Script::new(vec![redirect, json_reply(200, "[]")]);
    let github = github(script.clone());
    let response = github
        .request(Method::GET, "user/repos", &[], None)
        .await
        .unwrap();
    assert_eq!(response.status, 302);
    assert_eq!(
        response.headers.get("location").unwrap(),
        "https://elsewhere.example/next"
    );
    assert_eq!(script.seen().len(), 1);
}
