#![cfg(test)]

use rstest::rstest;
use serde_json::json;

use super::ClientKind;

#[rstest]
#[case::sync(ClientKind::Sync, "sync")]
#[case::asynchronous(ClientKind::Asynchronous, "async")]
fn writes_the_litellm_name(#[case] kind: ClientKind, #[case] name: &str) {
    assert_eq!(serde_json::to_value(kind).unwrap(), json!(name));
}

#[rstest]
#[case::sync(ClientKind::Sync, "sync")]
#[case::asynchronous(ClientKind::Asynchronous, "async")]
fn reads_the_litellm_name(#[case] kind: ClientKind, #[case] name: &str) {
    let parsed: ClientKind = serde_json::from_value(json!(name)).unwrap();
    assert_eq!(parsed, kind);
}

#[rstest]
#[case::sync(ClientKind::Sync, "sync")]
#[case::asynchronous(ClientKind::Asynchronous, "async")]
fn labels_the_class(#[case] kind: ClientKind, #[case] name: &str) {
    assert_eq!(kind.label(), name);
}
