#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};

use rstest::rstest;

use super::error::RegionError;
use super::{
    ModelRegion, ProviderRegionLists, RegionLists, RegionParams, get_model_region,
    infer_model_region,
};
use crate::catalog::{ModelCatalog, UnknownFields};
use crate::json_provider_registry::JSONProviderRegistry;
use crate::provider_defaults_a::policies::ProviderPolicies;
use crate::provider_defaults_b::globals::GlobalCredentials;
use crate::resolution::{ResolutionContext, ResolutionError};

#[derive(Clone, Copy, Default)]
struct Names<'name> {
    region_name: Option<&'name str>,
    vertex_location: Option<&'name str>,
    aws_region_name: Option<&'name str>,
    watsonx_region_name: Option<&'name str>,
}

fn in_region(region: &str) -> Names<'_> {
    Names {
        region_name: Some(region),
        vertex_location: None,
        aws_region_name: None,
        watsonx_region_name: None,
    }
}

fn region_lists(eu: &[&str], us: &[&str]) -> RegionLists {
    RegionLists {
        eu: eu.iter().map(|region| (*region).to_owned()).collect(),
        us: us.iter().map(|region| (*region).to_owned()).collect(),
    }
}

fn lists() -> ProviderRegionLists {
    ProviderRegionLists {
        azure: region_lists(&["europe", "central"], &["us"]),
        vertex_ai: region_lists(&["europe-"], &["us-"]),
        bedrock: region_lists(&["eu-"], &["us-"]),
        watsonx: region_lists(&["eu-"], &["us-"]),
    }
}

fn params<'params>(
    model: &'params str,
    custom_llm_provider: Option<&'params str>,
    names: Names<'params>,
) -> RegionParams<'params> {
    RegionParams {
        model,
        custom_llm_provider,
        api_base: None,
        api_key: None,
        use_litellm_proxy: false,
        region_name: names.region_name,
        vertex_location: names.vertex_location,
        aws_region_name: names.aws_region_name,
        watsonx_region_name: names.watsonx_region_name,
    }
}

fn empty_catalog() -> ModelCatalog {
    ModelCatalog::parse(b"{}", UnknownFields::Reject).unwrap()
}

fn infer(
    params: &RegionParams<'_>,
    default_vertex_location: Option<&str>,
) -> Result<Option<ModelRegion>, RegionError> {
    let catalog = empty_catalog();
    let known = BTreeSet::new();
    let custom = BTreeSet::new();
    let environment = BTreeMap::new();
    let context = ResolutionContext {
        catalog: &catalog,
        known_model_names: &known,
        custom_providers: &custom,
        environment: &environment,
        registry: JSONProviderRegistry::load(),
        policies: ProviderPolicies::default(),
        globals: GlobalCredentials::default(),
    };
    infer_model_region(params, &context, &lists(), default_vertex_location)
}

#[rstest]
#[case::vertex_location_wins_over_the_default(
    "vertex_ai",
    Names { vertex_location: Some("us-east1"), region_name: Some("eu"), ..Names::default() },
    Some("europe-west4"),
    Some("us-east1")
)]
#[case::vertex_default_when_no_location(
    "vertex_ai",
    Names { region_name: Some("eu"), ..Names::default() },
    Some("europe-west4"),
    Some("europe-west4")
)]
#[case::vertex_falls_back_to_region_name(
    "vertex_ai",
    Names { region_name: Some("eu"), ..Names::default() },
    None,
    Some("eu")
)]
#[case::vertex_empty_location_is_used_as_given(
    "vertex_ai",
    Names { vertex_location: Some(""), region_name: Some("eu"), ..Names::default() },
    Some("europe-west4"),
    Some("")
)]
#[case::vertex_ignores_the_aws_region(
    "vertex_ai",
    Names { aws_region_name: Some("us-east-1"), ..Names::default() },
    None,
    None
)]
#[case::bedrock_uses_aws_region_name(
    "bedrock",
    Names { aws_region_name: Some("eu-west-1"), region_name: Some("us"), ..Names::default() },
    None,
    Some("eu-west-1")
)]
#[case::bedrock_falls_back_to_region_name(
    "bedrock",
    Names { region_name: Some("us"), ..Names::default() },
    None,
    Some("us")
)]
#[case::bedrock_ignores_the_vertex_default("bedrock", Names::default(), Some("europe-west4"), None)]
#[case::watsonx_uses_its_own_region(
    "watsonx",
    Names { watsonx_region_name: Some("us-south"), region_name: Some("eu"), ..Names::default() },
    None,
    Some("us-south")
)]
#[case::watsonx_falls_back_to_region_name(
    "watsonx",
    Names { region_name: Some("eu"), ..Names::default() },
    None,
    Some("eu")
)]
#[case::other_providers_use_region_name(
    "azure",
    Names {
        region_name: Some("westeurope"),
        vertex_location: Some("us-east1"),
        aws_region_name: Some("us-east-1"),
        watsonx_region_name: Some("us-south"),
    },
    Some("europe-west4"),
    Some("westeurope")
)]
#[case::other_providers_without_a_region(
    "openai",
    Names { vertex_location: Some("us-east1"), ..Names::default() },
    Some("europe-west4"),
    None
)]
fn region_lookup_reads_the_providers_own_field(
    #[case] provider: &str,
    #[case] names: Names<'_>,
    #[case] default_vertex_location: Option<&str>,
    #[case] expected: Option<&str>,
) {
    let found = get_model_region(
        provider,
        &params("model", None, names),
        default_vertex_location,
    );
    assert_eq!(found, expected);
}

#[rstest]
#[case::azure_eu(
    "azure/gpt-4o",
    None,
    Names { region_name: Some("westeurope"), ..Names::default() },
    None,
    Some(ModelRegion::Eu)
)]
#[case::the_region_is_lowercased(
    "azure/gpt-4o",
    None,
    Names { region_name: Some("WestEurope"), ..Names::default() },
    None,
    Some(ModelRegion::Eu)
)]
#[case::azure_us(
    "azure/gpt-4o",
    None,
    Names { region_name: Some("EastUS2"), ..Names::default() },
    None,
    Some(ModelRegion::Us)
)]
#[case::the_eu_list_is_tested_before_the_us_list(
    "azure/gpt-4o",
    None,
    Names { region_name: Some("centralus"), ..Names::default() },
    None,
    Some(ModelRegion::Eu)
)]
#[case::a_region_in_neither_list(
    "azure/gpt-4o",
    None,
    Names { region_name: Some("mars"), ..Names::default() },
    None,
    None
)]
#[case::no_region_at_all("azure/gpt-4o", None, Names::default(), None, None)]
#[case::a_provider_without_lists(
    "openai/gpt-4o",
    None,
    Names { region_name: Some("europe"), ..Names::default() },
    None,
    None
)]
#[case::the_declared_provider_decides(
    "gpt-4o",
    Some("azure"),
    Names { region_name: Some("westeurope"), ..Names::default() },
    None,
    Some(ModelRegion::Eu)
)]
#[case::vertex_default_location(
    "vertex_ai/gemini-pro",
    None,
    Names::default(),
    Some("europe-west4"),
    Some(ModelRegion::Eu)
)]
#[case::vertex_location_over_default(
    "vertex_ai/gemini-pro",
    None,
    Names { vertex_location: Some("us-central1"), ..Names::default() },
    Some("europe-west4"),
    Some(ModelRegion::Us)
)]
#[case::bedrock_eu(
    "bedrock/claude",
    None,
    Names { aws_region_name: Some("eu-central-1"), ..Names::default() },
    None,
    Some(ModelRegion::Eu)
)]
#[case::watsonx_us(
    "watsonx/granite",
    None,
    Names { watsonx_region_name: Some("us-south"), ..Names::default() },
    None,
    Some(ModelRegion::Us)
)]
fn region_inference_tests_the_providers_lists(
    #[case] model: &str,
    #[case] custom_llm_provider: Option<&str>,
    #[case] names: Names<'_>,
    #[case] default_vertex_location: Option<&str>,
    #[case] expected: Option<ModelRegion>,
) {
    let found = infer(
        &params(model, custom_llm_provider, names),
        default_vertex_location,
    )
    .unwrap();
    assert_eq!(found, expected);
}

#[test]
fn region_inference_for_the_litellm_proxy_has_no_lists() {
    let names = in_region("westeurope");
    let mut proxied = params("azure/gpt-4o", None, names);
    proxied.use_litellm_proxy = true;
    assert_eq!(infer(&proxied, None).unwrap(), None);
}

#[test]
fn region_inference_fails_for_a_model_with_no_provider() {
    let names = in_region("westeurope");
    let error = infer(&params("no-such-model", None, names), None).unwrap_err();
    assert!(matches!(
        error,
        RegionError::Resolution(ResolutionError::MissingProvider { .. })
    ));
}

#[rstest]
#[case::eu(ModelRegion::Eu, "eu")]
#[case::us(ModelRegion::Us, "us")]
fn model_regions_print_in_lowercase(#[case] region: ModelRegion, #[case] expected: &str) {
    assert_eq!(region.to_string(), expected);
}
