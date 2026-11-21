use betterlite_config::sections::settings::settings_rules::{DbRow, UI_SETTINGS_FIELDS};
use betterlite_config::sections::settings::settings_store::SettingsStore;
use betterlite_config::ui_settings::UiSettingsDict;

pub enum GeneralSettingsTarget<'target> {
    Store(&'target mut SettingsStore),
    Map(&'target mut UiSettingsDict),
}

// apply_runtime_general_settings_flags: copy the ten runtime-gating fields and apply them as the ui_settings row.
#[must_use]
pub fn apply_runtime_ui_flags(
    settings: &UiSettingsDict,
    target: GeneralSettingsTarget<'_>,
) -> UiSettingsDict {
    let flags: UiSettingsDict = UI_SETTINGS_FIELDS
        .iter()
        .filter_map(|key| {
            settings
                .get(key)
                .map(|value| ((*key).to_owned(), value.clone()))
        })
        .collect();
    match target {
        GeneralSettingsTarget::Store(store) => {
            store.apply_db_row(DbRow::UiSettings, flags.clone());
        }
        GeneralSettingsTarget::Map(map) => map.extend(flags.clone()),
    }
    flags
}
