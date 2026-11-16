#![cfg(test)]

use rstest::rstest;

use super::{real_identity, user};
use crate::straiker_webhook::types::StraikerWebhookIdentity;

#[rstest]
#[case::email_first(
    r#"{"litellm_user_email":"a@x","litellm_user_id":"u1","end_user_id":"e"}"#,
    Some("a@x")
)]
#[case::user_id_without_email(r#"{"litellm_user_id":"u1","end_user_id":"e"}"#, Some("u1"))]
#[case::end_user_last(r#"{"end_user_id":"e"}"#, Some("e"))]
#[case::placeholder_user_id_skipped(
    r#"{"litellm_user_id":"default_user_id","end_user_id":"e"}"#,
    Some("e")
)]
#[case::master_key_email_skipped(
    r#"{"litellm_user_email":"litellm_proxy_master_key","litellm_user_id":"u1"}"#,
    Some("u1")
)]
#[case::only_placeholders(
    r#"{"litellm_user_id":"default_user_id","end_user_id":"litellm_proxy_master_key"}"#,
    None
)]
#[case::empty_strings_skipped(r#"{"litellm_user_email":"","litellm_user_id":"u1"}"#, Some("u1"))]
#[case::nobody("{}", None)]
fn picks_user(#[case] source: &str, #[case] expected: Option<&str>) {
    let identity: StraikerWebhookIdentity = serde_json::from_str(source).unwrap();
    assert_eq!(user(&identity).as_deref(), expected);
}

#[rstest]
#[case::person(Some("alice"), Some("alice"))]
#[case::empty(Some(""), None)]
#[case::default_user(Some("default_user_id"), None)]
#[case::master_key(Some("litellm_proxy_master_key"), None)]
#[case::absent(None, None)]
fn filters_placeholders(#[case] value: Option<&str>, #[case] expected: Option<&str>) {
    assert_eq!(real_identity(value), expected);
}
