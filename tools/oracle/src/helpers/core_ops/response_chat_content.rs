use betterlite_core::api::response_chat_content::ContentError;
use betterlite_core::api::response_chat_content::annotations::{
    AnnotationValue, annotation_fallback,
};
use betterlite_core::api::response_chat_content::content::{content_passthrough, text_by_role};
use betterlite_core::api::response_chat_content::image_file::image_and_file_fields;
use betterlite_core::api::response_chat_content::tool_choice::flat_tool_choice;
use serde::Deserialize;
use serde_json::{Map, Value, json};

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub(crate) enum Input {
    TextByRole {
        content: Value,
        role: String,
    },
    ContentPassthrough {
        items: Vec<Value>,
        role: String,
    },
    ImageAndFileFields {
        item: Map<String, Value>,
    },
    FlatToolChoice {
        choice: Value,
    },
    AnnotationFallback {
        annotations: Option<Vec<AnnotationInput>>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum AnnotationInput {
    Dict { value: Value },
    ModelDump { value: Value },
    ModelDumpAndDict { value: Value },
    LegacyDict { value: Value },
    Unsupported { name: String },
    Raises { error: String },
}

pub(crate) struct Op;
impl crate::helpers::Helper for Op {
    type Input = Input;
    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input {
            Input::TextByRole { content, role } => Ok(text_by_role(content, role)),
            Input::ContentPassthrough { items, role } => {
                content_passthrough(items, role)
                    .map(|projection| {
                        json!({"value": projection.value, "diagnostics": projection.diagnostics})
                    })
                    .map_err(|error| image_projection_error(&error))
            }
            Input::ImageAndFileFields { item } => {
                image_and_file_fields(item).map_err(|error| image_projection_error(&error))
            }
            Input::FlatToolChoice { choice } => Ok(flat_tool_choice(choice)),
            Input::AnnotationFallback { annotations } => {
                Ok(annotation_projection(annotations.as_deref()))
            }
        }
    }
}

fn image_projection_error(error: &ContentError) -> Box<crate::helpers::HelperError> {
    Box::new(crate::helpers::HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn annotation_projection(annotations: Option<&[AnnotationInput]>) -> Value {
    let values = annotations
        .unwrap_or_default()
        .iter()
        .map(|annotation| match annotation {
            AnnotationInput::Dict { value } => AnnotationValue::Dict(value.clone()),
            AnnotationInput::ModelDump { value } | AnnotationInput::ModelDumpAndDict { value } => {
                AnnotationValue::ModelDump(value.clone())
            }
            AnnotationInput::LegacyDict { value } => AnnotationValue::LegacyDict(value.clone()),
            AnnotationInput::Unsupported { name } => AnnotationValue::Unsupported(name.clone()),
            AnnotationInput::Raises { error } => AnnotationValue::Raises(error.clone()),
        })
        .collect::<Vec<_>>();
    let projection = annotation_fallback(annotations.map(|_| values.as_slice()));
    json!({"value": projection.value, "diagnostics": projection.diagnostics})
}
