mod dispatch;
mod fields;
mod metadata;
mod projection;
mod size;

use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;

use dispatch::body_dispatch;
use fields::{form_data, numeric};
use metadata::metadata;
use size::upload_size;

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum PartInput {
    Text {
        name: String,
        value: String,
    },
    Upload {
        name: String,
        filename: String,
        #[serde(default)]
        content_type: Option<String>,
        content: String,
    },
}

#[derive(Debug, Deserialize)]
pub(crate) struct Input {
    operation: String,
    #[serde(default)]
    parts: Vec<PartInput>,
    #[serde(default)]
    fields: Vec<(String, Json)>,
    #[serde(default)]
    annotations: BTreeMap<String, String>,
    #[serde(default)]
    prefix: Option<String>,
    #[serde(default)]
    file_size: Option<u64>,
    #[serde(default)]
    existing_metadata: BTreeMap<String, Json>,
    #[serde(default)]
    method: String,
    #[serde(default)]
    content_type: String,
    #[serde(default)]
    body: String,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let operation = input.operation.as_str();
        match operation {
            "form_data" => form_data(input),
            "numeric_form_coercion" => numeric(input),
            "nested_form_metadata" => metadata(input),
            "upload_size_unset" => upload_size(input),
            "request_body_dispatch" => body_dispatch(input),
            _other => Err(failure(
                "ValueError",
                format!("unknown request_form operation: {operation}"),
            )),
        }
    }
}

fn failure(class: &str, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: Some(None),
        param: Some(None),
        code: Some(None),
        message,
    })
}
