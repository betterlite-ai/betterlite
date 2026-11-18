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
use betterlite_router::deployment_identity::error::InvalidDeployment;
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use betterlite_router::model_catalog_lookup::alias::{AliasMap, AliasTarget};
use betterlite_router::model_catalog_lookup::error::LookupError;
use betterlite_router::model_catalog_lookup::lookup::CatalogLookup;
use betterlite_router::routing_groups::index::RoutingGroupIndex;
use betterlite_router::wildcard_catalog::Candidate;
use betterlite_router::wildcard_catalog::error::PatternCatalogError;

// The wildcard a fixture registers without the deployment that would back it, so a lookup of a name it matches fails.
const BROKEN_PATTERN: &str = "broken-*";

// The model the unbacked wildcard names; the missing deployment is the only defect the fixture adds.
const BROKEN_MODEL: &str = "openai/gpt-4o";

// One deployment a test declares, its model_info left as raw JSON for the non-mapping case.
pub struct Row<'row> {
    pub model_name: &'row str,
    pub litellm_params: &'row JsonObject,
    pub model_info: &'row Json,
}

// The inferred provider a fixture reads through its boxed closure.
type Provider = Box<dyn Fn(&str) -> Option<String>>;

// A Router as the model listing filters see it: the model list with its indices and the model_group_alias map.
pub struct Snapshot {
    pub catalog: DeploymentCatalog,
    pub aliases: AliasMap,
    pub groups: RoutingGroupIndex,
    pub provider: Provider,
}

impl Snapshot {
    // set_model_list: every row under the catalog's name and pattern indices, then model_group_alias beside them.
    pub fn build(rows: &[Row<'_>], aliases: &JsonObject) -> Result<Self, FixtureError> {
        let mut catalog = DeploymentCatalog::default();
        for row in rows {
            let spec = spec(row)?;
            catalog.index_patterns(&spec)?;
            catalog.push(spec);
            catalog.model_names.insert(row.model_name.to_owned());
        }
        Ok(Self {
            catalog,
            aliases: alias_map(aliases),
            groups: RoutingGroupIndex::default(),
            provider: provider(),
        })
    }

    // A wildcard the model list never backed: a listing of a name it matches must fail instead of serving it.
    pub fn with_unbacked_pattern(mut self) -> Result<Self, FixtureError> {
        let candidate = Candidate {
            model_id: Some(BROKEN_PATTERN.to_owned()),
            model: BROKEN_MODEL.to_owned(),
        };
        self.catalog
            .pattern_router
            .add_pattern(BROKEN_PATTERN, candidate)?;
        Ok(self)
    }

    // The lookup the listing filters read, over this snapshot.
    #[must_use]
    pub fn lookup(&self) -> CatalogLookup<'_> {
        CatalogLookup {
            catalog: &self.catalog,
            model_group_alias: &self.aliases,
            routing_groups: &self.groups,
            inferred_provider: &*self.provider,
        }
    }
}

// The fixture's own failure: a row or pattern the catalog refuses, or a lookup that runs off its indices.
#[derive(Debug, thiserror::Error)]
pub enum FixtureError {
    #[error(transparent)]
    Deployment(#[from] InvalidDeployment),
    #[error(transparent)]
    Pattern(#[from] PatternCatalogError),
    #[error(transparent)]
    Lookup(#[from] LookupError),
}

// set_model_list's per-row identity: the declared id, or the stable one it derives from the name and params.
fn spec(row: &Row<'_>) -> Result<DeploymentSpec, FixtureError> {
    let info = match row.model_info {
        Json::Object(info) => info.clone(),
        // a row whose model_info is not a mapping reads as one without a discoverable entry
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            JsonObject::new()
        }
    };
    Ok(DeploymentSpec::new(
        row.model_name.to_owned(),
        row.litellm_params.clone(),
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

mod tests;
