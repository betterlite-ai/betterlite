use betterlite_http::error::UrlPathError;
use betterlite_http::url_path::{encode_segment, encode_segments};

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UrlPathInput {
    pub(crate) function: UrlPathFunction,
    pub(crate) value: String,
    #[serde(default = "default_field_name")]
    pub(crate) field_name: String,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum UrlPathFunction {
    Segment,
    Segments,
}

pub(crate) fn invoke(input: &UrlPathInput) -> Result<serde_json::Value, Box<super::HelperError>> {
    let result = match input.function {
        UrlPathFunction::Segment => encode_segment(&input.value, &input.field_name),
        UrlPathFunction::Segments => encode_segments(&input.value, &input.field_name),
    };
    result
        .map(serde_json::Value::String)
        .map_err(|error| Box::new(helper_error(&error, &input.field_name)))
}

fn default_field_name() -> String {
    let field_name = "path parameter";
    field_name.to_owned()
}

fn helper_error(error: &UrlPathError, field_name: &str) -> super::HelperError {
    let message = match error {
        UrlPathError::Required { .. } => format!("{field_name} is required"),
        UrlPathError::DotSegment { .. } => {
            format!("{field_name} cannot be a dot path segment")
        }
    };
    super::HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    }
}
