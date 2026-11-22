use betterlite_bedrock::invoke_response::error::InvokeReplyError;
use serde_json::{Value, json};

use betterlite_oracle_helper_api::error::HelperError;

pub(super) fn value_error(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: String::from("ValueError"),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

pub(super) fn reply_error(error: &InvokeReplyError) -> Box<HelperError> {
    let Some(bedrock) = error.bedrock() else {
        return value_error(error.to_string());
    };
    Box::new(HelperError {
        class: String::from("BedrockError"),
        status: Some(bedrock.error.status_code),
        error_type: None,
        param: None,
        code: None,
        message: bedrock.error.message.clone(),
    })
}

pub(super) fn error_json(error: &HelperError) -> Value {
    json!({
        "error": {
            "class": error.class,
            "status": error.status,
        }
    })
}
