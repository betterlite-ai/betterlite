use indexmap::IndexMap;
use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

mod tests;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    router_settings: RawRouter,
}

#[derive(Debug, Default, Deserialize)]
struct RawRouter {
    model_group_alias: Option<IndexMap<String, RawAlias>>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawAlias {
    Target(String),
    Item {
        model: String,
        #[serde(default)]
        hidden: bool,
    },
}

// An alias with an empty model rewrites nothing: LiteLLM reads an unset os.environ/ reference as None and skips the entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupAlias {
    pub model: String,
    pub hidden: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    pub model_group_alias: IndexMap<String, GroupAlias>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let mut model_group_alias = IndexMap::new();
        for (name, raw) in self.router_settings.model_group_alias.unwrap_or_default() {
            let (written, hidden) = match raw {
                RawAlias::Target(model) => (model, false),
                RawAlias::Item { model, hidden } => (model, hidden),
            };
            let model = written
                .strip_prefix("os.environ/")
                .map(|variable| lookup.lookup(variable).unwrap_or_default())
                .unwrap_or(written);
            model_group_alias.insert(name, GroupAlias { model, hidden });
        }
        Ok(Settings { model_group_alias })
    }
}
