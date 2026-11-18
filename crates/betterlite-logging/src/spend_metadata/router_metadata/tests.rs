#![cfg(test)]

use betterlite_models::model_registration::JsonObject;
use rstest::rstest;

use super::router_metadata_for_spend_log;

fn object(text: &str) -> JsonObject {
    serde_json::from_str(text).expect("the fixture is a JSON object")
}

#[rstest]
#[case::no_metadata(None)]
#[case::no_model_info(Some(r#"{"model_info": null}"#))]
#[case::model_info_is_not_an_object(Some(r#"{"model_info": "router"}"#))]
#[case::missing_the_marker(Some(r#"{"model_info": {"id": "m"}}"#))]
#[case::marker_as_text(Some(r#"{"model_info": {"internal_router_model": "true"}}"#))]
#[case::marker_as_number(Some(r#"{"model_info": {"internal_router_model": 1}}"#))]
fn without_the_internal_router_marker_there_is_no_provenance(#[case] metadata: Option<&str>) {
    let parsed_metadata = metadata.map(object);
    assert_eq!(
        router_metadata_for_spend_log(
            parsed_metadata.as_ref(),
            Some("requested"),
            Some("selected"),
            Some("openai"),
            Some("correlation"),
        ),
        None
    );
}

#[test]
fn the_marker_carries_the_provenance_through() {
    let metadata = object(r#"{"model_info": {"internal_router_model": true}}"#);
    let router_metadata = router_metadata_for_spend_log(
        Some(&metadata),
        Some("requested"),
        Some("selected"),
        Some("openai"),
        Some("correlation"),
    )
    .expect("the marker is set");
    assert_eq!(
        router_metadata.requested_model.as_deref(),
        Some("requested")
    );
    assert_eq!(router_metadata.selected_model.as_deref(), Some("selected"));
    assert_eq!(router_metadata.selected_provider.as_deref(), Some("openai"));
    assert_eq!(
        router_metadata.router_correlation_id.as_deref(),
        Some("correlation")
    );
}

#[test]
fn empty_names_become_null_but_the_correlation_id_passes_through() {
    let metadata = object(r#"{"model_info": {"internal_router_model": true}}"#);
    let router_metadata =
        router_metadata_for_spend_log(Some(&metadata), Some(""), Some(""), Some(""), Some(""))
            .expect("the marker is set");
    assert_eq!(router_metadata.requested_model, None);
    assert_eq!(router_metadata.selected_model, None);
    assert_eq!(router_metadata.selected_provider, None);
    assert_eq!(router_metadata.router_correlation_id.as_deref(), Some(""));

    let absent = router_metadata_for_spend_log(Some(&metadata), None, None, None, None)
        .expect("the marker is set");
    assert_eq!(
        absent,
        super::super::records::SpendLogsRouterMetadata::default()
    );
}
