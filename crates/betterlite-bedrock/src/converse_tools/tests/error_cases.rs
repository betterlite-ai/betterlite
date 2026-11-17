use betterlite_anthropic::anthropic_capability_catalog::forced_tool::ForcedToolChoiceError;
use betterlite_openai::error::OpenAiError;
use rstest::rstest;

use crate::converse_tools::error::ConverseToolsError;

#[rstest]
#[case::format_not_an_object(ConverseToolsError::FormatNotAnObject)]
#[case::missing_format_type(ConverseToolsError::MissingFormatType)]
#[case::json_schema_not_an_object(ConverseToolsError::JsonSchemaNotAnObject)]
#[case::missing_schema_field(ConverseToolsError::MissingSchemaField)]
#[case::not_text(ConverseToolsError::NotText { field: "name" })]
#[case::tool_not_an_object(ConverseToolsError::ToolNotAnObject)]
#[case::tool_type(ConverseToolsError::ToolType)]
#[case::tool_function(ConverseToolsError::ToolFunction)]
#[case::tool_parameters(ConverseToolsError::ToolParameters)]
#[case::parameters_not_an_object(ConverseToolsError::ParametersNotAnObject)]
#[case::missing_parameter_type(ConverseToolsError::MissingParameterType)]
#[case::tool_choice_function(ConverseToolsError::ToolChoiceFunction)]
#[case::tool_choice_name(ConverseToolsError::ToolChoiceName)]
#[case::tools_not_list(ConverseToolsError::ToolsNotList)]
fn malformed_request_error_variants_answer_400(#[case] error: ConverseToolsError) {
    assert_eq!(error.status_code(), Some(400), "{error}");
}

#[test]
fn internal_error_variants_keep_their_status() {
    let append = ConverseToolsError::Append(OpenAiError::RefNotText);
    assert_eq!(append.status_code(), None, "{append}");
    let wire = serde_json::from_str::<u8>("not a number").expect_err("the fixture is not a byte");
    assert_eq!(ConverseToolsError::Wire(wire).status_code(), None);
}

#[test]
fn forced_tool_errors_keep_their_own_status() {
    let unsupported = ForcedToolChoiceError::Unsupported {
        model: "amazon.nova-pro-v1:0".to_owned(),
    };
    assert_eq!(
        ConverseToolsError::ForcedTool(unsupported).status_code(),
        Some(400)
    );
    assert_eq!(
        ConverseToolsError::ForcedTool(ForcedToolChoiceError::MissingType).status_code(),
        None
    );
}
