use betterlite_config::sections::vertex_credential_sources::VertexSettings;

use super::harness::{File, build, settings, store};
use super::{DONE, PROJECT_OPTIONS};

#[tokio::test]
async fn project_missing_project_fails_without_any_port_call() {
    let harness = build("{}", &VertexSettings::default(), None, None, &[]);
    let error = store(&harness, File::default())
        .await
        .expect_err("a missing project");
    assert_eq!(
        error.to_string(),
        "vertex_project is required for Vertex AI RAG ingestion. Set it in vector_store config."
    );
    assert!(harness.token.credentials_seen().is_empty());
    assert!(harness.http.calls().is_empty());
}

#[tokio::test]
async fn project_empty_project_option_is_missing() {
    let harness = build(
        r#"{"vertex_project":""}"#,
        &settings(Some("settings-proj"), None, None),
        None,
        None,
        &[],
    );
    let error = store(&harness, File::default())
        .await
        .expect_err("an empty project");
    assert_eq!(
        error.to_string(),
        "vertex_project is required for Vertex AI RAG ingestion. Set it in vector_store config."
    );
    assert!(harness.token.credentials_seen().is_empty());
}

#[tokio::test]
async fn project_settings_project_is_used_when_the_options_omit_one() {
    let harness = build(
        "{}",
        &settings(Some("settings-proj"), None, None),
        None,
        None,
        &[(200, DONE)],
    );
    store(&harness, File::default())
        .await
        .expect("a created corpus");
    assert_eq!(
        harness.http.calls()[0].url,
        "https://us-central1-aiplatform.googleapis.com/v1beta1/projects/settings-proj/locations/us-central1/ragCorpora"
    );
}

#[tokio::test]
async fn project_vertex_ai_alias_is_used_alone() {
    let options = r#"{"vertex_ai_project":"alias"}"#;
    let harness = build(
        options,
        &settings(None, None, None),
        None,
        None,
        &[(200, DONE)],
    );
    store(&harness, File::default())
        .await
        .expect("a created corpus");
    assert_eq!(
        harness.http.calls()[0].url,
        "https://us-central1-aiplatform.googleapis.com/v1beta1/projects/alias/locations/us-central1/ragCorpora"
    );
}

#[tokio::test]
async fn project_location_uses_the_settings_value() {
    let harness = build(
        PROJECT_OPTIONS,
        &settings(None, Some("europe-west4"), None),
        None,
        None,
        &[(200, DONE)],
    );
    store(&harness, File::default())
        .await
        .expect("a created corpus");
    assert_eq!(
        harness.http.calls()[0].url,
        "https://europe-west4-aiplatform.googleapis.com/v1beta1/projects/proj/locations/europe-west4/ragCorpora"
    );
}

#[tokio::test]
async fn project_location_uses_the_global_host() {
    let options = r#"{"vertex_project":"proj","vertex_location":"global"}"#;
    let harness = build(
        options,
        &VertexSettings::default(),
        None,
        None,
        &[(200, DONE)],
    );
    store(&harness, File::default())
        .await
        .expect("a created corpus");
    assert_eq!(
        harness.http.calls()[0].url,
        "https://aiplatform.googleapis.com/v1beta1/projects/proj/locations/global/ragCorpora"
    );
}

#[tokio::test]
async fn project_invalid_location_fails_after_the_token_call() {
    let options = r#"{"vertex_project":"proj","vertex_location":"Bad_Location"}"#;
    let harness = build(options, &VertexSettings::default(), None, None, &[]);
    let error = store(&harness, File::default())
        .await
        .expect_err("an invalid location");
    assert_eq!(error.to_string(), "invalid vertex_location format");
    assert_eq!(harness.token.credentials_seen().len(), 1);
    assert!(harness.http.calls().is_empty());
}
