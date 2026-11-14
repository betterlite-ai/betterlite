use indexmap::IndexMap;
use serde::Deserialize;
use serde::de::IgnoredAny;

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
    Item(RawItem),
}

// an untagged enum buffers its value, so the loader's ignored-key pass never sees a typo here; convert reports the extra keys
#[derive(Debug, Deserialize)]
struct RawItem {
    model: String,
    #[serde(default)]
    hidden: bool,
    #[serde(flatten)]
    unknown: IndexMap<String, IgnoredAny>,
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
        let mut paths = Vec::new();
        for (name, raw) in self.router_settings.model_group_alias.unwrap_or_default() {
            let (written, hidden) = match raw {
                RawAlias::Target(model) => (model, false),
                RawAlias::Item(item) => {
                    paths.extend(
                        item.unknown
                            .keys()
                            .map(|key| format!("router_settings.model_group_alias.{name}.{key}")),
                    );
                    (item.model, item.hidden)
                }
            };
            let model = written
                .strip_prefix("os.environ/")
                .map(|variable| lookup.lookup(variable).unwrap_or_default())
                .unwrap_or(written);
            model_group_alias.insert(name, GroupAlias { model, hidden });
        }
        if !paths.is_empty() {
            return Err(ConfigError::Unsupported { paths });
        }
        Ok(Settings { model_group_alias })
    }
}
