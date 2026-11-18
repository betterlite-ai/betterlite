#![cfg(test)]

use super::{obj, router, text};

#[test]
fn a_stored_project_and_location_return_their_credentials() {
    let mut resolver = router(vec![], &[]);
    resolver.add_vertex_credentials("project-one", "us-central1", Some(text("stored")));
    let stored = resolver
        .get_vertex_credentials(Some("project-one"), Some("us-central1"))
        .expect("stored credentials");
    assert_eq!(stored.vertex_credentials, Some(text("stored")));
    assert!(
        resolver
            .get_vertex_credentials(Some("project-two"), Some("us-central1"))
            .is_none()
    );
}

#[test]
fn a_missing_project_location_or_key_returns_the_default() {
    let mut resolver = router(vec![], &[]);
    resolver.add_vertex_credentials("project-one", "us-central1", Some(text("stored")));
    resolver
        .set_default_vertex_config(Some(obj(&[
            ("vertex_project", text("default-project")),
            ("vertex_location", text("default-location")),
        ])))
        .unwrap();
    let missing_key = resolver
        .get_vertex_credentials(Some("unknown"), Some("us-central1"))
        .expect("default on key miss");
    assert_eq!(
        missing_key.vertex_project.as_deref(),
        Some("default-project")
    );
    let absent = resolver
        .get_vertex_credentials(None, Some("us-central1"))
        .expect("default without a project");
    assert_eq!(absent.vertex_project.as_deref(), Some("default-project"));
    let stored = resolver
        .get_vertex_credentials(Some("project-one"), Some("us-central1"))
        .expect("stored credentials");
    assert_eq!(stored.vertex_project.as_deref(), Some("project-one"));
}
#[test]
fn location_and_empty_keys_and_replacement_are_preserved() {
    let mut resolver = router(vec![], &[]);
    resolver.add_vertex_credentials("", "", Some(text("first")));
    resolver.add_vertex_credentials("", "", Some(text("replacement")));
    let record = resolver.get_vertex_credentials(Some(""), Some("")).unwrap();
    assert_eq!(record.vertex_project.as_deref(), Some(""));
    assert_eq!(record.vertex_location.as_deref(), Some(""));
    assert_eq!(record.vertex_credentials, Some(text("replacement")));
    assert!(resolver.get_vertex_credentials(Some(""), None).is_none());
    assert!(
        resolver
            .get_vertex_credentials(Some(""), Some("elsewhere"))
            .is_none()
    );
}
