use betterlite_core::api::video_records::helpers::{
    // ast-grep-ignore: no-long-names
    extract_model_from_target_model_names,
    get_custom_provider_from_data,
    video_reference_to_id,
};
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    ExtractModel { value: Json },
    VideoReference { value: Json },
    CustomProvider { value: IndexMap<String, Json> },
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let answer = match input {
            Input::ExtractModel { value } => {
                // ast-grep-ignore: no-long-names
                extract_model_from_target_model_names(value).or_value_error()?
            }
            Input::VideoReference { value } => video_reference_to_id(value).or_value_error()?,
            Input::CustomProvider { value } => {
                get_custom_provider_from_data(value).or_value_error()?
            }
        };
        Ok(answer.map_or(Value::Null, Value::String))
    }
}
