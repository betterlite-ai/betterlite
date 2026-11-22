use betterlite_config::ui_settings::UiSettingsDict;
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::ui_settings_state::record::{UiSettingErrorKind, UiSettings};
use serde::{Deserialize, Serialize};

pub(crate) struct Op;

#[derive(Deserialize)]
pub(crate) struct Input {
    settings: UiSettingsDict,
}

#[derive(Serialize)]
pub(crate) struct Output {
    accepted: bool,
    error_type: Option<UiSettingErrorKind>,
    error_field: Option<&'static str>,
    error_index: Option<usize>,
    dump: Option<UiSettingsDict>,
    exclude_unset: Option<UiSettingsDict>,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Input) -> HelperResult {
        let output = match UiSettings::validate(&input.settings) {
            Ok(record) => Output {
                accepted: true,
                error_type: None,
                error_field: None,
                error_index: None,
                dump: Some(record.model_dump()),
                exclude_unset: Some(record.model_dump_exclude_unset()),
            },
            Err(error) => Output {
                accepted: false,
                error_type: Some(error.kind),
                error_field: Some(error.field),
                error_index: error.index,
                dump: None,
                exclude_unset: None,
            },
        };
        serde_json::to_value(output).or_value_error()
    }
}
