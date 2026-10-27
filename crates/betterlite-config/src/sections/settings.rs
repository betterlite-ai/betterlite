pub mod changed_section_keys;
pub mod coerce_bool;
pub mod settings_resolution;
pub mod settings_rules;
pub mod settings_store;

pub type FieldDescriptor = settings_resolution::FieldDescriptor;
pub type FieldSource = settings_resolution::FieldSource;
pub type DbRow = settings_rules::DbRow;
pub type JsonValue = settings_rules::JsonValue;
pub type Section = settings_rules::Section;
pub type ConfigOwnedKeyError = settings_store::ConfigOwnedKeyError;
pub type SettingsStore = settings_store::SettingsStore;

crate::sections::stub_section!();
