#![cfg(test)]

mod body_cases;
mod media;
mod request_cases;
mod support;
mod thinking_cases;

use betterlite_context::request_records::RequestRecordError;
use betterlite_prompts::bedrock_processor::error::ProcessorError;
use rstest::rstest;

use crate::converse_messages::error::ConverseMessageError;
use crate::converse_request::error::ConverseRequestError;
use crate::error::BedrockError;

// Every wrapper keeps the status its source error reports, and the provider-free ones carry none.
#[test]
fn converse_request_error_status_codes() {
    let rejected = ConverseRequestError::FieldType {
        field: "requestMetadata",
        expected: "an object",
    };
    assert_eq!(rejected.status_code(), Some(400));

    let forwarded = ConverseRequestError::Messages(ConverseMessageError::ToolCallsWithoutTools);
    assert_eq!(forwarded.status_code(), Some(400));

    let metadata = ConverseRequestError::Metadata(BedrockError::MetadataNotObject);
    assert_eq!(metadata.status_code(), Some(400));

    let internal = ConverseRequestError::Records(RequestRecordError::MetadataMembership {
        bucket: "params".to_owned(),
    });
    assert_eq!(internal.status_code(), None);

    let processor = ConverseRequestError::Processor(ProcessorError::NoMessages);
    assert_eq!(processor.status_code(), None);
}

#[rstest]
#[case::converse_body_shape(body_cases::converse_body_shape)]
#[case::tool_config_injection(body_cases::tool_config_injection)]
#[case::context_management_filter(body_cases::context_management_filter)]
#[case::system_message_order(request_cases::system_message_order)]
#[case::inline_document_image(request_cases::inline_document_image)]
#[case::model_arn_request(request_cases::model_arn_request)]
#[case::output_format_precedence(request_cases::output_format_precedence)]
#[case::thinking_history_policy(thinking_cases::thinking_history_policy)]
fn converse_request_cases(#[case] case: fn()) {
    case();
}
