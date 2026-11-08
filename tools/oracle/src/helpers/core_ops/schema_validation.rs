use betterlite_core::schema::normalize::{normalize_json_schema_types, normalize_tool_schema};
use betterlite_core::schema::validate::validate_schema;
use betterlite_core::schema::{SchemaError, SchemaJson, SchemaLimits};
use betterlite_error::error::ErrorClass;
use serde::Deserialize;

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let limits = SchemaLimits::default();
        let outcome = match input.mode {
            Mode::Normalize => normalize_json_schema_types(&input.schema, &limits),
            Mode::Tool => normalize_tool_schema(&input.tool, &limits),
            Mode::Validate => {
                validate_schema(&input.schema, &input.response, &limits).map(|()| SchemaJson::Null)
            }
        };
        outcome.map_err(|error| schema_failure(&error, &input.schema, &input.response))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    mode: Mode,
    #[serde(default)]
    schema: SchemaJson,
    #[serde(default)]
    tool: SchemaJson,
    #[serde(default)]
    response: String,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Mode {
    Normalize,
    Tool,
    Validate,
}

// LiteLLM raises JSONSchemaValidationError for a bad response and an uncaught jsonschema.SchemaError otherwise
fn schema_failure(
    error: &SchemaError,
    schema: &SchemaJson,
    response: &str,
) -> Box<crate::helpers::HelperError> {
    let class = error.classify(schema, response);
    let class_name = if matches!(&class, ErrorClass::JsonSchemaValidation(_)) {
        "JSONSchemaValidationError"
    } else {
        "SchemaError"
    };
    let status = class.metadata().status_code;
    Box::new(crate::helpers::HelperError {
        class: String::from(class_name),
        status: Some(status),
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
