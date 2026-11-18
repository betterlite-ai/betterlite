#![cfg(test)]

use super::{Json, PassThroughDeployment, available, obj, router, text};

fn with_credentials(model_name: &str, credentials: Json) -> PassThroughDeployment {
    available(
        model_name,
        &format!("vertex_ai/{model_name}"),
        &[
            ("vertex_project", text("shared-project")),
            ("vertex_location", text("us-central1")),
            ("vertex_credentials", credentials),
        ],
    )
}

#[test]
fn object_key_order_does_not_split_a_credential() {
    let first = with_credentials(
        "first",
        Json::Object(obj(&[
            ("client_email", text("service@example.test")),
            ("private_key", text("key")),
        ])),
    );
    let second = with_credentials(
        "second",
        Json::Object(obj(&[
            ("private_key", text("key")),
            ("client_email", text("service@example.test")),
        ])),
    );
    let resolver = router(vec![first, second], &[]);
    let unnamed = resolver
        .vertex_deployment_credentials(None)
        .expect("identical credentials");
    assert_eq!(unnamed.vertex_project.as_deref(), Some("shared-project"));
}

#[test]
fn a_different_object_credential_splits_the_deployments() {
    let first = with_credentials("first", Json::Object(obj(&[("private_key", text("one"))])));
    let second = with_credentials("second", Json::Object(obj(&[("private_key", text("two"))])));
    let resolver = router(vec![first, second], &[]);
    assert!(resolver.vertex_deployment_credentials(None).is_none());
}

#[test]
fn a_text_credential_never_matches_an_object_credential() {
    let first = with_credentials("first", text("credentials.json"));
    let second = with_credentials(
        "second",
        Json::Object(obj(&[("private_key", text("credentials.json"))])),
    );
    let resolver = router(vec![first, second], &[]);
    assert!(resolver.vertex_deployment_credentials(None).is_none());
}

#[test]
fn a_text_credential_compares_by_its_own_text() {
    let same = router(
        vec![
            with_credentials("first", text("credentials")),
            with_credentials("second", text("credentials")),
        ],
        &[],
    );
    assert!(same.vertex_deployment_credentials(None).is_some());
}
#[test]
fn serialized_object_text_is_a_distinct_identity() {
    let first = with_credentials("first", text("{\"key\": \"value\"}"));
    let second = with_credentials("second", Json::Object(obj(&[("key", text("value"))])));
    assert!(
        router(vec![first, second], &[])
            .vertex_deployment_credentials(None)
            .is_none()
    );
    let different = router(
        vec![
            with_credentials("one", text("a")),
            with_credentials("two", text("b")),
        ],
        &[],
    );
    assert!(different.vertex_deployment_credentials(None).is_none());
}
