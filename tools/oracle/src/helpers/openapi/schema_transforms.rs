use betterlite_openapi::inline_expansion::{InlineError, inline_request_body};
use betterlite_openapi::schema_transforms::tree::JsonObject;
use betterlite_openapi::schema_transforms::{
    add_request_body_to_paths, add_request_schema, add_schema_to_components,
};

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    AddSchemaToComponents {
        openapi: JsonObject,
        schema_name: String,
        schema: JsonObject,
    },
    AddRequestBodyToPaths {
        openapi: JsonObject,
        paths: Vec<String>,
        schema_ref: String,
    },
    AddRequestSchema {
        openapi: JsonObject,
        schema_name: String,
        schema: Option<JsonObject>,
        paths: Vec<String>,
    },
    InlineRequestBody {
        schema: Option<JsonObject>,
        example: JsonObject,
    },
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Input) -> crate::helpers::HelperResult {
        // LiteLLM indexes the definitions map, so a missing one surfaces as KeyError
        let (class, message) = match transform(input.clone()).map(serde_json::to_value) {
            Ok(Ok(value)) => return Ok(value),
            Ok(Err(error)) => ("SerializationError", error.to_string()),
            Err(error) => ("KeyError", error.to_string()),
        };
        Err(Box::new(crate::helpers::HelperError {
            class: class.to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message,
        }))
    }
}

fn transform(input: Input) -> Result<JsonObject, InlineError> {
    match input {
        Input::AddSchemaToComponents {
            mut openapi,
            schema_name,
            schema,
        } => {
            add_schema_to_components(&mut openapi, &schema_name, &schema);
            Ok(openapi)
        }
        Input::AddRequestBodyToPaths {
            mut openapi,
            paths,
            schema_ref,
        } => {
            let path_names = paths.iter().map(String::as_str).collect::<Vec<_>>();
            add_request_body_to_paths(&mut openapi, &path_names, &schema_ref);
            Ok(openapi)
        }
        Input::AddRequestSchema {
            mut openapi,
            schema_name,
            schema,
            paths,
        } => {
            let path_names = paths.iter().map(String::as_str).collect::<Vec<_>>();
            add_request_schema(&mut openapi, &schema_name, schema.as_ref(), &path_names);
            Ok(openapi)
        }
        Input::InlineRequestBody { schema, example } => {
            inline_request_body(schema.as_ref(), &example)
        }
    }
}
