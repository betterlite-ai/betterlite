use betterlite_http::url_path::{encode_segment, encode_segments};

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Operation {
    PathEncoding(PathEncoding),
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PathEncoding {
    pub(super) function: Function,
    pub(super) value: String,
    pub(super) field_name: String,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Function {
    Segment,
    Segments,
}

pub(super) fn run(operation: &Operation) -> crate::helpers::HelperResult {
    match operation {
        Operation::PathEncoding(input) => {
            let result = match input.function {
                Function::Segment => encode_segment(&input.value, &input.field_name),
                Function::Segments => encode_segments(&input.value, &input.field_name),
            };
            result.map(serde_json::Value::String).map_err(|error| {
                Box::new(crate::helpers::HelperError {
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
}
