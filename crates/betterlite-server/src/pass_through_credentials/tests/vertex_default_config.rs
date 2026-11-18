#![cfg(test)]

use rstest::rstest;

use super::{Json, obj, router, text};

#[test]
fn environment_variables_fill_the_default() {
    let mut resolver = router(
        vec![],
        &[
            ("DEFAULT_VERTEXAI_PROJECT", "environment-project"),
            ("DEFAULT_VERTEXAI_LOCATION", "environment-location"),
            (
                "DEFAULT_GOOGLE_APPLICATION_CREDENTIALS",
                "/environment.json",
            ),
        ],
    );
    resolver.set_default_vertex_config(None).unwrap();
    let config = resolver
        .get_vertex_credentials(None, None)
        .expect("environment default");
    assert_eq!(
        config.vertex_project.as_deref(),
        Some("environment-project")
    );
    assert_eq!(
        config.vertex_location.as_deref(),
        Some("environment-location")
    );
    assert_eq!(config.vertex_credentials, Some(text("/environment.json")));
}

#[test]
fn provided_values_resolve_os_environ_references() {
    let mut resolver = router(
        vec![],
        &[
            ("PROJECT", "resolved-project"),
            ("GOOGLE_CREDS", "resolved-credentials"),
        ],
    );
    resolver
        .set_default_vertex_config(Some(obj(&[
            ("vertex_project", text("os.environ/PROJECT")),
            ("vertex_location", text("us-central1")),
            ("vertex_credentials", text("os.environ/GOOGLE_CREDS")),
        ])))
        .unwrap();
    let config = resolver
        .get_vertex_credentials(None, None)
        .expect("provided default");
    assert_eq!(config.vertex_project.as_deref(), Some("resolved-project"));
    assert_eq!(config.vertex_location.as_deref(), Some("us-central1"));
    assert_eq!(
        config.vertex_credentials,
        Some(text("resolved-credentials"))
    );
}

#[test]
fn an_unset_reference_leaves_the_field_absent() {
    let mut resolver = router(vec![], &[("EMPTY", "")]);
    resolver
        .set_default_vertex_config(Some(obj(&[
            ("vertex_project", text("os.environ/MISSING")),
            ("vertex_location", text("os.environ/EMPTY")),
        ])))
        .unwrap();
    let config = resolver
        .get_vertex_credentials(None, None)
        .expect("provided default");
    assert_eq!(config.vertex_project, None);
    assert_eq!(config.vertex_location, None);
}

#[test]
fn empty_environment_variables_count_as_unset() {
    let mut resolver = router(
        vec![],
        &[
            ("DEFAULT_VERTEXAI_PROJECT", ""),
            ("DEFAULT_VERTEXAI_LOCATION", ""),
            ("DEFAULT_GOOGLE_APPLICATION_CREDENTIALS", ""),
        ],
    );
    resolver.set_default_vertex_config(None).unwrap();
    let config = resolver
        .get_vertex_credentials(None, None)
        .expect("empty default");
    assert_eq!(config.vertex_project, None);
    assert_eq!(config.vertex_location, None);
    assert_eq!(config.vertex_credentials, None);
}

#[test]
fn stored_credentials_beat_the_default_for_their_key() {
    let mut resolver = router(vec![], &[]);
    resolver
        .set_default_vertex_config(Some(obj(&[
            ("vertex_project", text("default-project")),
            ("vertex_location", text("default-location")),
        ])))
        .unwrap();
    resolver.add_vertex_credentials("other-project", "us-central1", None);
    let by_key = resolver
        .get_vertex_credentials(Some("other-project"), Some("us-central1"))
        .expect("stored key");
    assert_eq!(by_key.vertex_project.as_deref(), Some("other-project"));
    assert_eq!(by_key.vertex_credentials, None);
    let fallback = resolver
        .get_vertex_credentials(Some("missing-project"), Some("us-central1"))
        .expect("default on key miss");
    assert_eq!(fallback.vertex_project.as_deref(), Some("default-project"));
}
#[rstest]
#[case::project("vertex_project")]
#[case::location("vertex_location")]
#[case::credentials("vertex_credentials")]
#[case::unknown("unknown")]
fn invalid_config_keeps_the_previous_default(#[case] field: &str) {
    let mut resolver = router(vec![], &[]);
    resolver
        .set_default_vertex_config(Some(obj(&[("vertex_project", text("kept"))])))
        .unwrap();
    let error = resolver
        .set_default_vertex_config(Some(obj(&[(field, Json::Bool(true))])))
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        format!("invalid pass-through Vertex config field {field}")
    );
    assert_eq!(
        resolver
            .default_vertex_config
            .as_ref()
            .unwrap()
            .vertex_project
            .as_deref(),
        Some("kept")
    );
}

#[test]
fn explicit_null_empty_and_object_values_do_not_read_defaults() {
    let mut resolver = router(vec![], &[("DEFAULT_VERTEXAI_PROJECT", "ignored")]);
    resolver
        .set_default_vertex_config(Some(obj(&[
            ("vertex_project", Json::Null),
            ("vertex_location", text("")),
            (
                "vertex_credentials",
                Json::Object(obj(&[("key", text("value"))])),
            ),
        ])))
        .unwrap();
    let record = resolver.default_vertex_config.as_ref().unwrap();
    assert!(record.vertex_project.is_none());
    assert_eq!(record.vertex_location.as_deref(), Some(""));
    assert_eq!(
        record.vertex_credentials,
        Some(Json::Object(obj(&[("key", text("value"))])))
    );
    resolver.set_default_vertex_config(Some(obj(&[]))).unwrap();
    assert!(
        resolver
            .default_vertex_config
            .as_ref()
            .unwrap()
            .vertex_location
            .is_none()
    );
}
