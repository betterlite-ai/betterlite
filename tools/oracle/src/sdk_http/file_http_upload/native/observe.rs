use betterlite_core::api::file_http_upload::config::FileResponse;
use betterlite_core::api::file_http_upload::error::UploadError;
use betterlite_core::api::vector_store_http_write::error::BaseLlmException;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Number;

use crate::sdk_http::file_http_upload::{Diagnostics, Failure, Outcome};

use super::super::super::error::SdkHttpError;
use super::{ADDITIONAL_ARGS, PLACEHOLDER};

pub(super) fn file_object(response: &FileResponse) -> Json {
    let mut object = IndexMap::new();
    object.insert(String::from("id"), Json::Text(String::from("file-oracle")));
    object.insert(String::from("object"), Json::Text(String::from("file")));
    object.insert(
        String::from("bytes"),
        Json::Number(Number::from(response.body.len())),
    );
    object.insert(
        String::from("created_at"),
        Json::Number(Number::from(0_u32)),
    );
    object.insert(
        String::from("filename"),
        Json::Text(String::from("oracle.txt")),
    );
    object.insert(String::from("purpose"), Json::Text(String::from("batch")));
    Json::Object(object)
}

pub(super) fn diagnostics(state: &LoggingState, origin: &str) -> Diagnostics {
    let details = &state.model_call_details;
    Diagnostics {
        input: observed(details, "input", origin),
        api_key: observed(details, "api_key", origin),
        api_base: argument(details, "api_base", origin),
        headers: argument(details, "headers", origin),
        complete_input_dict: argument(details, "complete_input_dict", origin),
    }
}

fn observed(details: &IndexMap<String, Json>, name: &str, origin: &str) -> Json {
    match details.get(name) {
        Some(value) => unalias(value.clone(), origin),
        None => Json::Null,
    }
}

fn argument(details: &IndexMap<String, Json>, name: &str, origin: &str) -> Json {
    let Some(Json::Object(args)) = details.get(ADDITIONAL_ARGS) else {
        return Json::Null;
    };
    observed(args, name, origin)
}

pub(super) fn outcome(
    result: Result<Json, UploadError>,
    diagnostics: Option<Diagnostics>,
    origin: &str,
) -> Result<Outcome, SdkHttpError> {
    match result {
        Ok(value) => Ok(Outcome {
            result: Some(unalias(value, origin)),
            failure: None,
            diagnostics,
        }),
        Err(UploadError::Provider(error)) => Ok(Outcome {
            result: None,
            failure: Some(failure(error, origin)),
            diagnostics,
        }),
        Err(other) => Err(SdkHttpError::Invalid(other.to_string())),
    }
}

fn failure(error: BaseLlmException, origin: &str) -> Failure {
    Failure {
        status: error.status_code,
        synthesized: error.status_code_is_synthesized,
        message: error.message,
        headers: error
            .headers
            .into_iter()
            .map(|(name, value)| (name, unalias(value, origin)))
            .collect(),
    }
}

// the mock's origin differs per side, so every observed string spells it as the placeholder
fn unalias(value: Json, origin: &str) -> Json {
    match value {
        Json::Text(text) => Json::Text(text.replace(origin, PLACEHOLDER)),
        Json::List(items) => Json::List(
            items
                .into_iter()
                .map(|item| unalias(item, origin))
                .collect(),
        ),
        Json::Object(fields) => Json::Object(
            fields
                .into_iter()
                .map(|(name, value)| (name, unalias(value, origin)))
                .collect(),
        ),
        Json::Null => Json::Null,
        Json::Bool(flag) => Json::Bool(flag),
        Json::Number(number) => Json::Number(number),
    }
}
