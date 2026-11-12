use std::path::PathBuf;

use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::batch_records::{
    BatchFileContent, BatchRecords, BatchRecordsError, BatchRecordsInput, BatchRecordsResult,
    ReplacedContent, ReplacedFile,
};
use betterlite_core::routes::CallType;
use betterlite_openai::files::records::FilesPurpose;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum FileInput {
    Utf8 { text: String },
    Hex { hex: String },
    Path { path: String },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    ParseJsonl {
        content: String,
    },
    ReplaceModel {
        file: FileInput,
        new_model_name: String,
    },
    ShouldReplaceModel {
        purpose: FilesPurpose,
        passthrough: bool,
    },
    RouterMetadataName {
        function_name: Option<String>,
    },
    IsBatchRetrieveCallType {
        call_type: CallType,
    },
    ValidateListLimit {
        limit: Option<i64>,
    },
}

fn invalid(message: String) -> Box<crate::helpers::HelperError> {
    Box::new(crate::helpers::HelperError {
        class: String::from("InvalidInput"),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn file_content(file: &FileInput) -> Result<BatchFileContent, String> {
    match file {
        FileInput::Utf8 { text } => Ok(BatchFileContent::Bytes(text.clone().into_bytes())),
        FileInput::Hex { hex } => hex::decode(hex)
            .map(BatchFileContent::Bytes)
            .map_err(|error| error.to_string()),
        FileInput::Path { path } => Ok(BatchFileContent::Path(PathBuf::from(path))),
    }
}

fn replaced_value(replaced: ReplacedFile) -> Result<Value, String> {
    match replaced.content {
        ReplacedContent::Original => {
            Ok(json!({"kind": "original", "logged_preview": replaced.logged_preview}))
        }
        ReplacedContent::Rewritten(output) => {
            let text = String::from_utf8(output.content).map_err(|error| error.to_string())?;
            let rows = text
                .split('\n')
                .map(serde_json::from_str::<Value>)
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| error.to_string())?;
            Ok(json!({
                "kind": "rewritten",
                "rows": rows,
                "name": output.name,
                "content_type": output.content_type,
                "logged_preview": replaced.logged_preview,
            }))
        }
    }
}

fn result_value(outcome: Result<BatchRecordsResult, BatchRecordsError>) -> Result<Value, String> {
    match outcome {
        Ok(BatchRecordsResult::Rows(rows)) => Ok(json!({"rows": rows, "logged_preview": null})),
        Ok(BatchRecordsResult::Replaced(replaced)) => replaced_value(replaced),
        Ok(BatchRecordsResult::ShouldReplace(flag) | BatchRecordsResult::BatchRetrieve(flag)) => {
            Ok(Value::Bool(flag))
        }
        Ok(BatchRecordsResult::MetadataName(name)) => Ok(Value::String(name.to_owned())),
        Ok(BatchRecordsResult::ListLimitAccepted) => Ok(json!({"accepted": true})),
        Err(BatchRecordsError::Unparsed(tail)) => {
            Ok(json!({"rows": null, "logged_preview": tail.preview}))
        }
        Err(BatchRecordsError::ListLimit(exception)) => Ok(json!({
            "accepted": false,
            "message": exception.message,
            "type": exception.error_type,
            "param": exception.param,
            "code": exception.code,
            "openai_code": exception.openai_code,
            "headers": exception.headers,
        })),
        Err(BatchRecordsError::Decode(error)) => Err(error.to_string()),
        Err(BatchRecordsError::Encode(error)) => Err(error.to_string()),
    }
}

fn service_input(input: &Input) -> Result<BatchRecordsInput, String> {
    Ok(match input {
        Input::ParseJsonl { content } => BatchRecordsInput::ParseJsonl {
            content: content.clone().into_bytes(),
        },
        Input::ReplaceModel {
            file,
            new_model_name,
        } => BatchRecordsInput::ReplaceModel {
            file: file_content(file)?,
            new_model_name: new_model_name.clone(),
        },
        Input::ShouldReplaceModel {
            purpose,
            passthrough,
        } => BatchRecordsInput::ShouldReplaceModel {
            purpose: *purpose,
            passthrough: *passthrough,
        },
        Input::RouterMetadataName { function_name } => BatchRecordsInput::RouterMetadataName {
            function_name: function_name.clone(),
        },
        Input::IsBatchRetrieveCallType { call_type } => {
            BatchRecordsInput::IsBatchRetrieveCallType {
                call_type: *call_type,
            }
        }
        Input::ValidateListLimit { limit } => {
            BatchRecordsInput::ValidateListLimit { limit: *limit }
        }
    })
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let service = BatchRecords {
            limits: ApiLimits::default(),
        };
        service_input(input)
            .and_then(|request| result_value(service.run(request)))
            .map_err(invalid)
    }
}
