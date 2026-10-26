pub mod config_models;
pub mod enums;
pub mod item;

#[derive(Debug, Default, serde::Deserialize)]
pub struct Raw;

#[derive(Debug, Clone, Default)]
pub struct Settings;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, _lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(Settings)
    }
}

#[path = "guardrail_identity/guardrail_base_init_kwargs/mod.rs"]
mod guardrail_base_init_kwargs;
#[path = "guardrail_identity/guardrail_config_models/mod.rs"]
mod guardrail_config_models;
#[path = "guardrail_identity/guardrail_enums/mod.rs"]
mod guardrail_enums;
#[path = "guardrail_identity/guardrail_item_defaults/mod.rs"]
mod guardrail_item_defaults;
#[path = "guardrail_identity/pii_entity_categories/mod.rs"]
mod pii_entity_categories;
