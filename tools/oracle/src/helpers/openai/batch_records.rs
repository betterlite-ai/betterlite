use std::path::PathBuf;

use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::batch_records::{
    BatchFileContent, BatchRecords, BatchRecordsError, BatchRecordsInput, BatchRecordsResult,
    ReplacedContent, ReplacedFile,
};
use betterlite_core::routes::CallType;
use betterlite_error::proxy_exception::ProxyException;
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

fn rewritten_rows(content: Vec<u8>) -> Result<Vec<Value>, String> {
    let text = String::from_utf8(content).map_err(|error| error.to_string())?;
    text.split('\n')
        .map(serde_json::from_str::<Value>)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())
}

fn replaced_value(replaced: ReplacedFile) -> Result<Value, String> {
    let ReplacedFile {
        content,
        logged_preview,
    } = replaced;
    let ReplacedContent::Rewritten(output) = content else {
        return Ok(json!({"kind": "original", "logged_preview": logged_preview}));
    };
    let rows = rewritten_rows(output.content)?;
    Ok(json!({
        "kind": "rewritten",
        "rows": rows,
        "name": output.name,
        "content_type": output.content_type,
        "logged_preview": logged_preview,
    }))
}

fn limit_error_value(exception: &ProxyException) -> Value {
    json!({
        "accepted": false,
        "message": exception.message,
        "type": exception.error_type,
        "param": exception.param,
        "code": exception.code,
        "openai_code": exception.openai_code,
        "headers": exception.headers,
    })
}

fn result_value(outcome: Result<BatchRecordsResult, BatchRecordsError>) -> Result<Value, String> {
    let value = match outcome {
        Ok(BatchRecordsResult::Rows(rows)) => json!({"rows": rows, "logged_preview": null}),
        Ok(BatchRecordsResult::Replaced(replaced)) => replaced_value(replaced)?,
        Ok(BatchRecordsResult::ShouldReplace(flag) | BatchRecordsResult::BatchRetrieve(flag)) => {
            Value::Bool(flag)
        }
        Ok(BatchRecordsResult::MetadataName(name)) => Value::String(name.to_owned()),
        Ok(BatchRecordsResult::ListLimitAccepted) => json!({"accepted": true}),
        Err(BatchRecordsError::Unparsed(tail)) => {
            json!({"rows": null, "logged_preview": tail.preview})
        }
        Err(BatchRecordsError::ListLimit(exception)) => limit_error_value(&exception),
        Err(BatchRecordsError::Decode(source)) => return Err(source.to_string()),
        Err(BatchRecordsError::Encode(source)) => return Err(source.to_string()),
    };
    Ok(value)
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
