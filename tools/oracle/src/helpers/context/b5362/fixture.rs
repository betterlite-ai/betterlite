use betterlite_context::non_default_params::Params;
use betterlite_context::non_default_params::error::MapperError;
use betterlite_context::non_default_params::mapper::OptionalParamsMapper;
use betterlite_models::controls::json_value::JsonValue;
use serde::Deserialize;

// The Python helper builds the same mapper, so both sides copy the params and fail with the same status.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct MapperSpec {
    supported: Vec<String>,
    fail_status: Option<u16>,
    #[serde(default)]
    records: bool,
}

impl OptionalParamsMapper for MapperSpec {
    fn supported_openai_params(&self, _model: &str) -> Vec<String> {
        self.supported.clone()
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
                message: "fixture mapper failed".to_owned(),
            });
        }
        optional_params.extend(non_default_params.clone());
        if self.records {
            optional_params.insert("seen_drop".to_owned(), JsonValue::Bool(drop_params));
            optional_params.insert("seen_model".to_owned(), JsonValue::Text(model.to_owned()));
        }
        Ok(optional_params)
    }
}
