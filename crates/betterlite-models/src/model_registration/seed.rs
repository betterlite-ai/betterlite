use indexmap::IndexMap;

use super::builtin::{CACHE_PRICING_FIELDS, builtin_cost_entry, is_generalized_model_info};
use super::error::{EntryError, RegistrationWarning};
use super::{CostMapFetcher, JsonObject, ModelInfoLookup, RegistrationEnvironment};
use crate::catalog_generalizations::CatalogGeneralizations;
use crate::decisions::Json;
use crate::identity::declared::OAUTH_VIA_PROVIDER_INFO;
use crate::model_catalog_keys::ModelCostKeys;

pub(super) struct Incoming<'entry> {
    pub(super) key: &'entry str,
    pub(super) provider: &'entry str,
    pub(super) value: &'entry JsonObject,
    pub(super) display_name: &'entry str,
}

pub(super) struct Seed {
    pub(super) existing: JsonObject,
    pub(super) model_cost_key: String,
    pub(super) warning: Option<RegistrationWarning>,
}

fn declared(value: &JsonObject, field: &str) -> bool {
    value
        .get(field)
        .is_some_and(|present| !matches!(present, Json::Null))
}

fn authenticates_on_provider_info(incoming: &Incoming<'_>) -> bool {
    OAUTH_VIA_PROVIDER_INFO.iter().any(|id| {
        let slug = id.to_string();
        incoming.provider == slug
            || incoming
                .key
                .strip_prefix(&slug)
                .is_some_and(|rest| rest.starts_with('/'))
    })
}

fn rule_defaults(
    generalizations: &CatalogGeneralizations,
    key: &str,
) -> Result<JsonObject, EntryError> {
    let mut defaults = IndexMap::new();
    for (field, raw) in generalizations.match_capabilities(key).unwrap_or_default() {
        let parsed =
            serde_json::from_str::<Json>(raw.get()).map_err(|source| EntryError::RuleInfoJson {
                model: key.to_owned(),
                source,
            })?;
        defaults.insert(field, parsed);
    }
    Ok(defaults)
}

fn lacks_cache_pricing(value: &JsonObject) -> bool {
    !declared(value, "cache_creation_input_token_cost")
        && !declared(value, "cache_read_input_token_cost")
        && !declared(value, "tiered_pricing")
        && (declared(value, "input_cost_per_token") || declared(value, "output_cost_per_token"))
}

// A price the caller declares overrides the inherited one in update_dictionary, so only a missing one shows.
fn inherit_cache_pricing(existing: &mut JsonObject, builtin: &JsonObject) {
    for field in CACHE_PRICING_FIELDS {
        if let Some(price) = builtin
            .get(field)
            .filter(|found| !matches!(found, Json::Null))
        {
            existing.insert(field.to_owned(), price.clone());
        }
    }
}

pub(super) async fn seed_entry<Lookup, Fetcher>(
    model_cost: &JsonObject,
    keys: &mut ModelCostKeys,
    incoming: &Incoming<'_>,
    environment: &RegistrationEnvironment<'_, Lookup, Fetcher>,
) -> Result<Seed, EntryError>
where
    Lookup: ModelInfoLookup,
    Fetcher: CostMapFetcher,
{
    if authenticates_on_provider_info(incoming) {
        let existing = match model_cost.get(incoming.key) {
            Some(Json::Object(live)) => live.clone(),
            _ => IndexMap::new(),
        };
        return Ok(Seed {
            existing,
            model_cost_key: incoming.key.to_owned(),
            warning: None,
        });
    }
    let looked_up = environment
        .lookup
        .get_model_info(incoming.key, model_cost, keys)
        .await;
    let builtin_info = looked_up
        .filter(|info| !is_generalized_model_info(model_cost, info, environment.generalizations));
    if let Some(info) = builtin_info {
        let Some(Json::Text(model_cost_key)) = info.get("key") else {
            return Err(EntryError::BuiltinInfoWithoutKey {
                model: incoming.key.to_owned(),
            });
        };
        return Ok(Seed {
            model_cost_key: model_cost_key.clone(),
            existing: info,
            warning: None,
        });
    }
    seed_from_rules(model_cost, incoming, environment)
}

fn seed_from_rules<Lookup, Fetcher>(
    model_cost: &JsonObject,
    incoming: &Incoming<'_>,
    environment: &RegistrationEnvironment<'_, Lookup, Fetcher>,
) -> Result<Seed, EntryError> {
    let mut existing = rule_defaults(environment.generalizations, incoming.key)?;
    let builtin = builtin_cost_entry(
        model_cost,
        incoming.key,
        incoming.provider,
        environment.bedrock_base_model,
    );
    let warning = builtin.map_or_else(
        || {
            lacks_cache_pricing(incoming.value).then(|| RegistrationWarning::MissingCachePricing {
                model: incoming.display_name.to_owned(),
            })
        },
        |entry| {
            inherit_cache_pricing(&mut existing, &entry);
            None
        },
    );
    Ok(Seed {
        existing,
        model_cost_key: incoming.key.to_owned(),
        warning,
    })
}
