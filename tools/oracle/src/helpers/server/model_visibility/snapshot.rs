use std::collections::{BTreeMap, BTreeSet};

use betterlite_models::catalog::BundledCatalog;
use betterlite_models::decisions::Json;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::model_registration::JsonObject;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_models::resolution::inferred::inferred_provider;
use betterlite_router::catalog_updates::catalog::DeploymentCatalog;
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use betterlite_router::model_catalog_lookup::alias::{AliasMap, AliasTarget};
use betterlite_router::routing_groups::index::RoutingGroupIndex;
use betterlite_router::wildcard_catalog::Candidate;

use super::error::Failure;
use super::input::{Input, RouterCall};

// The hidden wildcard a broken_pattern case registers without the deployment that would back it.
const BROKEN_PATTERN: &str = "broken-*";

// The inferred provider a case reads through the snapshot's boxed closure.
pub(crate) type Provider = Box<dyn Fn(&str) -> Option<String>>;

// A Router as model_visibility sees it: the model list with its indices and the model_group_alias map.
pub(crate) struct Snapshot {
    pub(crate) catalog: DeploymentCatalog,
    pub(crate) aliases: AliasMap,
    pub(crate) groups: RoutingGroupIndex,
    pub(crate) provider: Provider,
}

impl Snapshot {
    // set_model_list: every row under the catalog's name and pattern indices, then the aliases beside them.
    pub(crate) fn build(input: &Input) -> Result<Self, Failure> {
        let mut catalog = DeploymentCatalog::default();
        for row in &input.model_list {
            let spec = spec(&row.model_name, &row.litellm_params, &row.model_info)?;
            catalog.index_patterns(&spec)?;
            catalog.push(spec);
            catalog.model_names.insert(row.model_name.clone());
        }
        if matches!(input.arguments.router, RouterCall::BrokenPattern) {
            let candidate = Candidate {
                model_id: Some(BROKEN_PATTERN.to_owned()),
                model: "openai/gpt-4o".to_owned(),
            };
            catalog
                .pattern_router
                .add_pattern(BROKEN_PATTERN, candidate)?;
        }
        Ok(Self {
            catalog,
            aliases: alias_map(&input.model_group_alias),
            groups: RoutingGroupIndex::default(),
            provider: provider(),
        })
    }
}

// set_model_list's per-row identity: the declared id, or the stable one it derives from the name and params.
pub(crate) fn spec(
    model_name: &str,
    litellm_params: &JsonObject,
    model_info: &Json,
) -> Result<DeploymentSpec, Failure> {
    let info = match model_info {
        Json::Object(info) => info.clone(),
        // a row whose model_info is not a mapping reads as one without a discoverable entry
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            JsonObject::new()
        }
    };
    Ok(DeploymentSpec::new(
        model_name.to_owned(),
        litellm_params.clone(),
        info,
        JsonObject::new(),
        String::new,
    )?)
}

// model_group_alias as the lookup reads it, keeping configuration order.
fn alias_map(entries: &JsonObject) -> AliasMap {
    entries
        .iter()
        .map(|(alias, value)| (alias.clone(), alias_target(value)))
        .collect()
}

fn alias_target(value: &Json) -> AliasTarget {
    match value {
        Json::Text(target) => AliasTarget::Name(target.clone()),
        Json::Object(item) => AliasTarget::Item {
            model: item.get("model").cloned(),
            hidden: item.get("hidden").cloned(),
        },
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) => {
            AliasTarget::Unsupported(value.clone())
        }
    }
}

// get_llm_provider over the bundled cost map, with the literal "None" prefix the source's f-string builds for an unknown provider.
fn provider() -> Provider {
    let costs = BundledCatalog::default().get();
    let registry = JSONProviderRegistry::load();
    Box::new(move |model: &str| {
        let environment = BTreeMap::new();
        let known = BTreeSet::new();
        let custom = BTreeSet::new();
        let context = ResolutionContext {
            catalog: costs.as_ref(),
            known_model_names: &known,
            custom_providers: &custom,
            environment: &environment,
            registry: registry.clone(),
            policies: ProviderPolicies::default(),
            globals: GlobalCredentials::default(),
        };
        let slug = inferred_provider(model, &context)
            .map_or_else(|| "None".to_owned(), |identity| identity.slug().to_owned());
        Some(slug)
    })
}
