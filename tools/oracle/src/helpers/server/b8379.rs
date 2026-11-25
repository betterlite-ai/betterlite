use betterlite::db_config_overlay::scrub::scrub_remote_modules;
use betterlite::db_config_overlay::state::settings_mapping;
use betterlite_config::sections::settings::{DbRow, JsonValue, Section, SettingsStore};
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;

#[derive(Debug, thiserror::Error)]
enum Failure {
    #[error("{0} is not a section the overlay scrubs")]
    UnknownSection(String),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    section: String,
    row: JsonValue,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        answer(input).or_value_error()
    }
}

// get_config with no config file and one stored row: the section the reload serves once the row is scrubbed
fn answer(input: &Input) -> Result<JsonValue, Failure> {
    let section = match input.section.as_str() {
        "general_settings" => Section::GeneralSettings,
        "litellm_settings" => Section::LitellmSettings,
        other => return Err(Failure::UnknownSection(other.to_owned())),
    };
    let scrubbed = scrub_remote_modules(section, &input.row);
    let mut store = SettingsStore::from(section);
    store.apply_db_row(DbRow::from(section), settings_mapping(Some(&scrubbed)));
    Ok(store.resolved().into_iter().collect())
}
