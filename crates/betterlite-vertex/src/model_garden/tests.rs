#![cfg(test)]

use rstest::rstest;

use super::{ModelGardenTargetInput, model_garden_target};
use crate::model_urls::error::VertexUrlError;

#[rstest]
#[case::prefixed_endpoint_id(
    "openai/5464397967697903616",
    "https://us-central1-aiplatform.googleapis.com/v1beta1/projects/proj/locations/us-central1/endpoints/5464397967697903616",
    ""
)]
#[case::bare_endpoint_id(
    "5464397967697903616",
    "https://us-central1-aiplatform.googleapis.com/v1beta1/projects/proj/locations/us-central1/endpoints/5464397967697903616",
    ""
)]
#[case::publisher_model(
    "meta/llama-3.3-70b-instruct-maas",
    "https://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/endpoints/openapi",
    "meta/llama-3.3-70b-instruct-maas"
)]
#[case::prefixed_publisher_model(
    "openai/meta/llama-3.3-70b-instruct-maas",
    "https://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/endpoints/openapi",
    "meta/llama-3.3-70b-instruct-maas"
)]
fn targets_select_the_openapi_or_beta_path(
    #[case] model: &str,
    #[case] expected_base: &str,
    #[case] expected_model: &str,
) {
    let target = model_garden_target(&ModelGardenTargetInput {
        model,
        api_base: None,
        vertex_location: None,
        vertex_project: Some("proj"),
        resolved_project: "auth-proj",
    })
    .unwrap();
    assert_eq!(target.api_base, expected_base);
    assert_eq!(target.model, expected_model);
}

#[rstest]
#[case::absent(
    None,
    "https://us-central1-aiplatform.googleapis.com/v1beta1/projects/proj/locations/us-central1/endpoints/123"
)]
#[case::explicit_default(
    Some("us-central1"),
    "https://us-central1-aiplatform.googleapis.com/v1beta1/projects/proj/locations/us-central1/endpoints/123"
)]
#[case::geography(
    Some("us"),
    "https://aiplatform.us.rep.googleapis.com/v1beta1/projects/proj/locations/us/endpoints/123"
)]
#[case::region(
    Some("europe-west1"),
    "https://europe-west1-aiplatform.googleapis.com/v1beta1/projects/proj/locations/europe-west1/endpoints/123"
)]
fn locations_select_the_host_and_default_to_us_central1(
    #[case] location: Option<&str>,
    #[case] expected_base: &str,
) {
    let target = model_garden_target(&ModelGardenTargetInput {
        model: "123",
        api_base: None,
        vertex_location: location,
        vertex_project: Some("proj"),
        resolved_project: "auth-proj",
    })
    .unwrap();
    assert_eq!(target.api_base, expected_base);
}

#[test]
fn the_requested_project_wins_over_the_resolved_project() {
    let target = model_garden_target(&ModelGardenTargetInput {
        model: "123",
        api_base: None,
        vertex_location: None,
        vertex_project: Some("requested"),
        resolved_project: "resolved",
    })
    .unwrap();
    assert_eq!(
        target.api_base,
        "https://us-central1-aiplatform.googleapis.com/v1beta1/projects/requested/locations/us-central1/endpoints/123"
    );
}

#[test]
fn the_resolved_project_is_used_when_none_is_requested() {
    let target = model_garden_target(&ModelGardenTargetInput {
        model: "123",
        api_base: None,
        vertex_location: None,
        vertex_project: None,
        resolved_project: "resolved",
    })
    .unwrap();
    assert_eq!(
        target.api_base,
        "https://us-central1-aiplatform.googleapis.com/v1beta1/projects/resolved/locations/us-central1/endpoints/123"
    );
}

#[test]
fn an_empty_requested_project_is_used_as_given() {
    let target = model_garden_target(&ModelGardenTargetInput {
        model: "123",
        api_base: None,
        vertex_location: None,
        vertex_project: Some(""),
        resolved_project: "resolved",
    })
    .unwrap();
    assert_eq!(
        target.api_base,
        "https://us-central1-aiplatform.googleapis.com/v1beta1/projects//locations/us-central1/endpoints/123"
    );
}

#[rstest]
#[case::empty("")]
#[case::spaces("not a location")]
#[case::injection("us-central1.evil.example")]
fn a_bad_location_is_rejected(#[case] location: &str) {
    let result = model_garden_target(&ModelGardenTargetInput {
        model: "123",
        api_base: None,
        vertex_location: Some(location),
        vertex_project: Some("proj"),
        resolved_project: "auth-proj",
    });
    assert!(matches!(result, Err(VertexUrlError::InvalidLocation)));
}

#[rstest]
#[case::endpoint_id("openai/123", "")]
#[case::publisher_model("meta/llama-3.3-70b-instruct-maas", "meta/llama-3.3-70b-instruct-maas")]
fn an_explicit_base_is_kept(#[case] model: &str, #[case] expected_model: &str) {
    let target = model_garden_target(&ModelGardenTargetInput {
        model,
        api_base: Some("https://dedicated.example/v1beta1/projects/p/locations/l/endpoints/123"),
        vertex_location: Some("us-central1"),
        vertex_project: Some("proj"),
        resolved_project: "auth-proj",
    })
    .unwrap();
    assert_eq!(
        target.api_base,
        "https://dedicated.example/v1beta1/projects/p/locations/l/endpoints/123"
    );
    assert_eq!(target.model, expected_model);
}

#[test]
fn an_explicit_base_skips_location_validation() {
    let target = model_garden_target(&ModelGardenTargetInput {
        model: "123",
        api_base: Some("https://dedicated.example/v1"),
        vertex_location: Some("not a location"),
        vertex_project: None,
        resolved_project: "",
    })
    .unwrap();
    assert_eq!(target.api_base, "https://dedicated.example/v1");
}
