#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};

use rstest::rstest;

use super::inferred_provider;
use crate::catalog::{ModelCatalog, UnknownFields};
use crate::identity::provider_id::ProviderId;
use crate::json_provider_registry::JSONProviderRegistry;
use crate::provider_defaults_a::policies::ProviderPolicies;
use crate::provider_defaults_b::globals::GlobalCredentials;
use crate::resolution::{ProviderIdentity, ResolutionContext};

const CATALOG: &str =
    r#"{"gpt-4o": {"litellm_provider": "openai"}, "claude-2": {"litellm_provider": "anthropic"}}"#;

fn context<'context>(
    catalog: &'context ModelCatalog,
    known: &'context BTreeSet<String>,
    environment: &'context BTreeMap<String, String>,
    custom: &'context BTreeSet<String>,
) -> ResolutionContext<'context> {
    ResolutionContext {
        catalog,
        known_model_names: known,
        custom_providers: custom,
        environment,
        registry: JSONProviderRegistry::default(),
        policies: ProviderPolicies::default(),
        globals: GlobalCredentials::default(),
    }
}

#[rstest]
#[case::an_empty_name_has_no_provider("", None)]
#[case::a_known_name_gets_its_catalog_provider("gpt-4o", Some(ProviderId::Openai))]
#[case::an_anthropic_text_name_is_reported("claude-2", Some(ProviderId::AnthropicText))]
#[case::an_unknown_name_has_no_provider("brand-new-unknown", None)]
#[case::github_copilot_keeps_its_declaration(
    "github_copilot/gpt-4o",
    Some(ProviderId::GithubCopilot)
)]
#[case::chatgpt_keeps_its_declaration("chatgpt/gpt-4o", Some(ProviderId::Chatgpt))]
fn provider_resolution_tests_behavior(#[case] model: &str, #[case] expected: Option<ProviderId>) {
    let catalog = ModelCatalog::parse(CATALOG.as_bytes(), UnknownFields::Reject).unwrap();
    let environment = BTreeMap::new();
    let custom = BTreeSet::new();
    let known = BTreeSet::new();
    let observed = inferred_provider(model, &context(&catalog, &known, &environment, &custom));
    assert_eq!(observed, expected.map(ProviderIdentity::Known));
}
