#![cfg(test)]

use rstest::rstest;

use super::{
    CHAT_STANDARD_PARAMETER_NAMES, EMBEDDING_STANDARD_NAMES, ParameterClass,
    RESERVED_PARAMETER_NAMES, classify_parameter,
};
use crate::params::{CallType, TYPED_PARAMETER_NAMES};

#[rstest]
#[case::chat_standard(CallType::ChatCompletion, "tools", ParameterClass::Standard)]
#[case::chat_reasoning_effort(
    CallType::ChatCompletion,
    "reasoning_effort",
    ParameterClass::Standard
)]
#[case::chat_metadata(CallType::ChatCompletion, "metadata", ParameterClass::Standard)]
#[case::chat_thinking(CallType::ChatCompletion, "thinking", ParameterClass::Reserved)]
#[case::unknown_vendor_field(
    CallType::ChatCompletion,
    "vendor_flag",
    ParameterClass::ProviderPassthrough
)]
#[case::embedding_dimensions(CallType::Embeddings, "dimensions", ParameterClass::Standard)]
#[case::embedding_encoding_format(
    CallType::Embeddings,
    "encoding_format",
    ParameterClass::Standard
)]
#[case::embedding_input_is_content(CallType::Embeddings, "input", ParameterClass::Reserved)]
#[case::chat_dimensions_is_not_standard(
    CallType::ChatCompletion,
    "dimensions",
    ParameterClass::ProviderPassthrough
)]
#[case::chat_prompt_is_content(CallType::ChatCompletion, "prompt", ParameterClass::Reserved)]
#[case::chat_system_is_content(CallType::ChatCompletion, "system", ParameterClass::Reserved)]
#[case::chat_messages_is_content(CallType::ChatCompletion, "messages", ParameterClass::Reserved)]
#[case::chat_model_is_routing(CallType::ChatCompletion, "model", ParameterClass::Reserved)]
#[case::chat_typed_temperature(CallType::ChatCompletion, "temperature", ParameterClass::Reserved)]
#[case::chat_typed_n(CallType::ChatCompletion, "n", ParameterClass::Reserved)]
#[case::chat_drop_params_is_a_control(
    CallType::ChatCompletion,
    "drop_params",
    ParameterClass::Reserved
)]
#[case::chat_allowed_openai_params_is_a_control(
    CallType::ChatCompletion,
    "allowed_openai_params",
    ParameterClass::Reserved
)]
#[case::chat_additional_drop_params_is_a_control(
    CallType::ChatCompletion,
    "additional_drop_params",
    ParameterClass::Reserved
)]
#[case::chat_api_key_is_a_credential(CallType::ChatCompletion, "api_key", ParameterClass::Reserved)]
#[case::chat_api_base_is_a_credential(
    CallType::ChatCompletion,
    "api_base",
    ParameterClass::Reserved
)]
#[case::chat_max_retries_is_a_control(
    CallType::ChatCompletion,
    "max_retries",
    ParameterClass::Reserved
)]
#[case::chat_extra_headers_is_a_control(
    CallType::ChatCompletion,
    "extra_headers",
    ParameterClass::Reserved
)]
#[case::chat_litellm_metadata_is_internal(
    CallType::ChatCompletion,
    "litellm_metadata",
    ParameterClass::Reserved
)]
#[case::chat_mock_response_is_a_control(
    CallType::ChatCompletion,
    "mock_response",
    ParameterClass::Reserved
)]
fn provider_params_tests_behavior_table(
    #[case] call_type: CallType,
    #[case] name: &str,
    #[case] expected: ParameterClass,
) {
    assert_eq!(classify_parameter(name, call_type), expected);
}

fn typed_names_are_chat_standard_names() {
    assert!(
        TYPED_PARAMETER_NAMES
            .iter()
            .all(|name| CHAT_STANDARD_PARAMETER_NAMES.contains(name))
    );
}

fn embedding_standard_names_are_validated_embeddings_names() {
    assert_eq!(
        EMBEDDING_STANDARD_NAMES,
        &["dimensions", "encoding_format", "user"]
    );
}

fn reserved_names_are_not_standard_names() {
    assert!(
        RESERVED_PARAMETER_NAMES
            .iter()
            .all(|name| !EMBEDDING_STANDARD_NAMES.contains(name))
    );
}

#[rstest]
#[case::typed_names_are_chat_standard_names(typed_names_are_chat_standard_names)]
#[case::embedding_standard_names_are_validated_embeddings_names(
    embedding_standard_names_are_validated_embeddings_names
)]
#[case::reserved_names_are_not_standard_names(reserved_names_are_not_standard_names)]
fn provider_params_tests_behavior(#[case] scenario: fn()) {
    scenario();
}
