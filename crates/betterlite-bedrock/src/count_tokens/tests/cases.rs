use rstest::rstest;

#[derive(Debug, Clone, Copy)]
enum Scenario {
    TypedBlockInvokeModel,
    UntypedMessagesConverse,
    ToolNameSchema,
    CountEndpointPath,
    CountToAnthropicResult,
    InvokeBodyPythonJsonSpacing,
    InvokeBodyDefaults,
    ConverseContentSystem,
    CountModelMissing,
    CountMessagesEmpty,
    CountMessageNotObject,
    CountMessageRoleMissing,
    CountMessageContentMissing,
    CountInvokeModelOnly,
    EmptyToolsOmission,
}

#[rstest]
#[case::typed_block_invoke_model(Scenario::TypedBlockInvokeModel)]
#[case::untyped_messages_converse(Scenario::UntypedMessagesConverse)]
#[case::tool_name_schema(Scenario::ToolNameSchema)]
#[case::count_endpoint_path(Scenario::CountEndpointPath)]
#[case::count_to_anthropic_result(Scenario::CountToAnthropicResult)]
#[case::invoke_body_python_json_spacing(Scenario::InvokeBodyPythonJsonSpacing)]
#[case::invoke_body_defaults(Scenario::InvokeBodyDefaults)]
#[case::converse_content_system(Scenario::ConverseContentSystem)]
#[case::count_model_missing(Scenario::CountModelMissing)]
#[case::count_messages_empty(Scenario::CountMessagesEmpty)]
#[case::count_message_not_object(Scenario::CountMessageNotObject)]
#[case::count_message_role_missing(Scenario::CountMessageRoleMissing)]
#[case::count_message_content_missing(Scenario::CountMessageContentMissing)]
#[case::count_invoke_model_only(Scenario::CountInvokeModelOnly)]
#[case::empty_tools_omission(Scenario::EmptyToolsOmission)]
fn count_tokens_cases(#[case] scenario: Scenario) {
    match scenario {
        Scenario::TypedBlockInvokeModel => super::transform_cases::typed_block_invoke_model(),
        Scenario::UntypedMessagesConverse => super::transform_cases::untyped_messages_converse(),
        Scenario::ToolNameSchema => super::transform_cases::tool_name_schema(),
        Scenario::CountEndpointPath => super::endpoint_cases::count_endpoint_path(),
        Scenario::CountToAnthropicResult => super::response_cases::count_to_anthropic_result(),
        Scenario::InvokeBodyPythonJsonSpacing => {
            super::transform_cases::invoke_body_python_json_spacing();
        }
        Scenario::InvokeBodyDefaults => super::transform_cases::invoke_body_defaults(),
        Scenario::ConverseContentSystem => super::transform_cases::converse_content_system(),
        Scenario::CountModelMissing => super::error_cases::count_model_missing(),
        Scenario::CountMessagesEmpty => super::error_cases::count_messages_empty(),
        Scenario::CountMessageNotObject => super::error_cases::count_message_not_object(),
        Scenario::CountMessageRoleMissing => super::error_cases::count_message_role_missing(),
        Scenario::CountMessageContentMissing => super::error_cases::count_message_content_missing(),
        Scenario::CountInvokeModelOnly => super::error_cases::count_invoke_model_only(),
        Scenario::EmptyToolsOmission => super::transform_cases::empty_tools_omission(),
    }
}
