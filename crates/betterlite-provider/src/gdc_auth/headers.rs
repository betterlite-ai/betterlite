use std::collections::BTreeMap;

use secrecy::{ExposeSecret as _, SecretString};

pub type Headers = BTreeMap<String, SecretString>;

const AUTHORIZATION: &str = "Authorization";
const CONTENT_TYPE: &str = "Content-Type";
const QUOTA_PROJECT: &str = "x-goog-user-project";

// The source projection looks the exact name up in a dict, so a lowercase authorization never suppresses this one
#[rustfmt::skip]
pub fn set_bearer_token(headers: &mut Headers, api_key: &SecretString) {
    if !headers.contains_key(AUTHORIZATION) {
        headers.insert(AUTHORIZATION.to_owned(), SecretString::from(format!("Bearer {}", api_key.expose_secret())));
    }
}
#[rustfmt::skip]
pub fn set_service_token(headers: &mut Headers, token: &SecretString) {
    headers.insert(
        AUTHORIZATION.to_owned(),
        SecretString::from(format!("Bearer {}", token.expose_secret())),
    );
}
#[rustfmt::skip]
pub fn set_content_type(headers: &mut Headers) {
    if !headers.contains_key("content-type") && !headers.contains_key(CONTENT_TYPE) {
        headers.insert(CONTENT_TYPE.to_owned(), SecretString::from("application/json".to_owned()));
    }
}
// Every case variant of the quota project header is dropped before the lowercase one is set
#[rustfmt::skip]
pub fn set_quota_project(headers: &mut Headers, project: &str) {
    headers.retain(|name, _| !name.eq_ignore_ascii_case(QUOTA_PROJECT));
    headers.insert(QUOTA_PROJECT.to_owned(), SecretString::from(format!("projects/{project}")));
}
