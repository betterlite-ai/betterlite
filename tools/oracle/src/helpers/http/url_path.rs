use betterlite_http::url_path::{encode_segment, encode_segments};
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) function: UrlPathFunction,
    pub(crate) value: String,
    pub(crate) field_name: String,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum UrlPathFunction {
    Segment,
    Segments,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Input) -> HelperResult {
        let result = match input.function {
            UrlPathFunction::Segment => encode_segment(&input.value, &input.field_name),
            UrlPathFunction::Segments => encode_segments(&input.value, &input.field_name),
        };
        result.map(serde_json::Value::String).map_err(|error| {
            Box::new(HelperError {
                class: "ValueError".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })
    }
}
