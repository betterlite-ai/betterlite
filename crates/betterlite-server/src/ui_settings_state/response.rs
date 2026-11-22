use std::collections::BTreeMap;

use betterlite_config::sections::settings::settings_resolution::FieldSource;
use betterlite_config::ui_settings::UiSettingsDict;
use serde::Serialize;

use super::schema::UiSettingsSchema;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UiSettingsResponse {
    pub values: UiSettingsDict,
    pub field_schema: UiSettingsSchema,
    pub source: BTreeMap<String, FieldSource>,
}
