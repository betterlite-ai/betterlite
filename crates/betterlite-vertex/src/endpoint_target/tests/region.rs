#![cfg(test)]

use rstest::rstest;

use crate::endpoint_target::error::EndpointTargetError;
use crate::endpoint_target::region::{DEFAULT_LOCATION, select_region};
use crate::endpoint_target::target::get_api_base;
use crate::model_urls::error::VertexUrlError;

use super::support::{MODEL, catalog};

#[derive(Debug, Clone, Copy)]
enum RegionCase {
    AbsentWithoutRegions,
    AbsentWithRegions,
    ConfiguredSupported,
    ConfiguredUnsupported,
    PrefixedModel,
    KnownModelWithoutRegions,
}

#[rstest]
#[case::absent_without_regions(RegionCase::AbsentWithoutRegions)]
#[case::absent_with_regions(RegionCase::AbsentWithRegions)]
#[case::configured_supported(RegionCase::ConfiguredSupported)]
#[case::configured_unsupported(RegionCase::ConfiguredUnsupported)]
#[case::prefixed_model(RegionCase::PrefixedModel)]
#[case::known_model_without_regions(RegionCase::KnownModelWithoutRegions)]
fn region_cases(#[case] scenario: RegionCase) {
    let catalog = catalog();
    match scenario {
        RegionCase::AbsentWithoutRegions => {
            let selection = select_region(&catalog, MODEL, None);
            assert_eq!(selection.region, DEFAULT_LOCATION);
            assert_eq!(selection.unsupported, None);
        }
        RegionCase::AbsentWithRegions => {
            let selection = select_region(&catalog, "deepseek-ai/deepseek-v3.1-maas", None);
            assert_eq!(selection.region, "us-central1");
            assert_eq!(selection.unsupported, None);
        }
        RegionCase::ConfiguredSupported => {
            let selection = select_region(
                &catalog,
                "qwen/qwen3-235b-a22b-instruct-2507-maas",
                Some("us-south1"),
            );
            assert_eq!(selection.region, "us-south1");
            assert_eq!(selection.unsupported, None);
        }
        RegionCase::ConfiguredUnsupported => {
            let selection = select_region(
                &catalog,
                "qwen/qwen3-235b-a22b-instruct-2507-maas",
                Some("europe-west1"),
            );
            assert_eq!(selection.region, "global");
            let unsupported = selection.unsupported.expect("the region is unsupported");
            assert_eq!(unsupported.configured, "europe-west1");
            assert_eq!(unsupported.supported, vec!["global", "us-south1"]);
        }
        RegionCase::PrefixedModel => {
            let selection = select_region(
                &catalog,
                "vertex_ai/deepseek-ai/deepseek-v3.1-maas",
                Some("global"),
            );
            assert_eq!(selection.region, "us-central1");
            assert!(selection.unsupported.is_some());
        }
        RegionCase::KnownModelWithoutRegions => {
            let selection = select_region(&catalog, "text-embedding-005", Some("europe-west1"));
            assert_eq!(selection.region, "europe-west1");
            assert_eq!(selection.unsupported, None);
        }
    }
}

#[rstest]
#[case::custom(
    Some("https://proxy.example.com/v1"),
    Some("europe-west1"),
    "https://proxy.example.com/v1"
)]
#[case::empty_custom(
    Some(""),
    Some("europe-west1"),
    "https://europe-west1-aiplatform.googleapis.com"
)]
#[case::absent_default(Some(""), None, "https://us-central1-aiplatform.googleapis.com")]
#[case::global(None, Some("global"), "https://aiplatform.googleapis.com")]
#[case::geography(None, Some("us"), "https://aiplatform.us.rep.googleapis.com")]
fn api_base_cases(
    #[case] api_base: Option<&str>,
    #[case] location: Option<&str>,
    #[case] expected: &str,
) {
    assert_eq!(get_api_base(api_base, location).unwrap(), expected);
}

#[test]
fn api_base_rejects_an_invalid_location() {
    let error = get_api_base(None, Some("bad loc!")).unwrap_err();
    assert!(matches!(
        error,
        EndpointTargetError::VertexUrl(VertexUrlError::InvalidLocation)
    ));
}

#[test]
fn api_base_skips_location_validation_with_a_custom_base() {
    assert_eq!(
        get_api_base(Some("https://proxy.example.com"), Some("bad loc!")).unwrap(),
        "https://proxy.example.com"
    );
}
