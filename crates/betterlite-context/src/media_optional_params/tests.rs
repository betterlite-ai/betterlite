#![cfg(test)]

use betterlite_models::controls::json_value::JsonValue;

use crate::non_default_params::Params;
use crate::non_default_params::error::MapperError;
use crate::non_default_params::mapper::OptionalParamsMapper;

mod image_generation;
mod transcription;

fn params(text: &str) -> Params {
    serde_json::from_str(text).unwrap()
}

// Copies the mapped params and records the model and drop flag it was called with.
struct FixtureMapper {
    supported: &'static [&'static str],
    fail_status: Option<u16>,
}

impl OptionalParamsMapper for FixtureMapper {
    fn supported_openai_params(&self, _model: &str) -> Vec<String> {
        self.supported.iter().map(ToString::to_string).collect()
    }

    fn map_openai_params(
        &self,
        non_default_params: &Params,
        mut optional_params: Params,
        model: &str,
        drop_params: bool,
    ) -> Result<Params, MapperError> {
        if let Some(status_code) = self.fail_status {
            return Err(MapperError {
                status_code,
                message: "mapper failed".to_owned(),
            });
        }
        optional_params.extend(non_default_params.clone());
        optional_params.insert("seen_drop".to_owned(), JsonValue::Bool(drop_params));
        optional_params.insert("seen_model".to_owned(), JsonValue::Text(model.to_owned()));
        Ok(optional_params)
    }
}
