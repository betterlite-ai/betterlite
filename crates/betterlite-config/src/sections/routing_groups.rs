use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::raw::RawNumber;
use crate::resolve::{resolve_number, resolve_string};
use crate::sections::Section;

mod tests;

// RouterConfig.routing_groups: absent or null means no group, an explicit empty list stays as given.
#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    router_settings: RawRouter,
}

#[derive(Debug, Default, Deserialize)]
struct RawRouter {
    routing_groups: Option<Vec<RawGroup>>,
}

#[derive(Debug, Deserialize)]
struct RawGroup {
    group_name: String,
    models: Vec<String>,
    routing_strategy: String,
    routing_strategy_args: Option<IndexMap<String, Json>>,
    model_priorities: Option<BTreeMap<String, RawNumber>>,
}

// One typed entry of router_settings.routing_groups, with every os.environ/ value already resolved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoutingGroupSettings {
    pub group_name: String,
    pub models: Vec<String>,
    pub routing_strategy: String,
    pub routing_strategy_args: Option<IndexMap<String, Json>>,
    pub model_priorities: Option<BTreeMap<String, u64>>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Settings {
    pub routing_groups: Option<Vec<RoutingGroupSettings>>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let Some(groups) = self.router_settings.routing_groups else {
            return Ok(Settings::default());
        };
        let mut converted = Vec::with_capacity(groups.len());
        for (index, group) in groups.into_iter().enumerate() {
            converted.push(group_settings(index, group, lookup)?);
        }
        Ok(Settings {
            routing_groups: Some(converted),
        })
    }
}

fn group_settings<L>(
    index: usize,
    group: RawGroup,
    lookup: &L,
) -> Result<RoutingGroupSettings, ConfigError>
where
    L: EnvironmentLookup,
{
    let path = |field: &str| format!("router_settings.routing_groups.{index}.{field}");
    let mut models = Vec::with_capacity(group.models.len());
    for (position, model) in group.models.into_iter().enumerate() {
        models.push(resolve_string(
            model,
            &format!("router_settings.routing_groups.{index}.models.{position}"),
            lookup,
        )?);
    }
    Ok(RoutingGroupSettings {
        group_name: resolve_string(group.group_name, &path("group_name"), lookup)?,
        models,
        routing_strategy: resolve_string(
            group.routing_strategy,
            &path("routing_strategy"),
            lookup,
        )?,
        routing_strategy_args: group.routing_strategy_args,
        model_priorities: priorities(index, group.model_priorities, lookup)?,
    })
}

fn priorities<L>(
    index: usize,
    priorities: Option<BTreeMap<String, RawNumber>>,
    lookup: &L,
) -> Result<Option<BTreeMap<String, u64>>, ConfigError>
where
    L: EnvironmentLookup,
{
    let Some(raw_priorities) = priorities else {
        return Ok(None);
    };
    let mut resolved = BTreeMap::new();
    for (model, priority) in raw_priorities {
        let path = format!("router_settings.routing_groups.{index}.model_priorities.{model}");
        // an empty variable counts as unset, so that model keeps no priority
        if let Some(value) = resolve_number(Some(priority), &path, lookup, parse_priority)? {
            resolved.insert(model, value);
        }
    }
    Ok(Some(resolved))
}

fn parse_priority(text: &str, path: &str) -> Result<u64, ConfigError> {
    text.parse::<u64>()
        .map_err(|source| ConfigError::InvalidInteger {
            path: path.to_owned(),
            source,
        })
}
