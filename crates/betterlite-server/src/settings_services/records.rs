use std::collections::BTreeMap;

use betterlite_config::sections::settings::JsonValue;

use super::schema::RootSchema;

// What a caller hands the shared services: the validated record as model_dump() renders it, its pinned pydantic-v2 schema and its declared field defaults; the update holds it across awaits, so the record is Sync
pub trait SettingsRecord: Sync {
    fn schema(&self) -> &RootSchema;

    fn values(&self) -> BTreeMap<String, JsonValue>;

    // _model_field_default: a model field's declared default, absent when the field has none
    fn field_default(&self, field_name: &str) -> Option<JsonValue>;
}
