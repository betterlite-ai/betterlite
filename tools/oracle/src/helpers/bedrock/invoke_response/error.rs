use betterlite_bedrock::invoke_response::error::InvokeReplyError;

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
    let mut helper = value_error(bedrock.error.message.clone());
    "BedrockError".clone_into(&mut helper.class);
    helper.status = Some(bedrock.error.status_code);
    helper
}
