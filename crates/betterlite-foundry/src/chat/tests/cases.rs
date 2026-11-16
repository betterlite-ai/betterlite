use rstest::rstest;

use super::{capabilities, environment, fields, messages, provider, request, response, url};

#[rstest]
#[case::models_chat_completions_route(url::models_chat_completions_route)]
#[case::api_version_url_precedence(url::api_version_url_precedence)]
#[case::strip_non_openai_message_fields(messages::strip_non_openai_message_fields)]
#[case::text_list_copy_behavior(messages::text_list_copy_behavior)]
#[case::extra_body_precedence(request::extra_body_precedence)]
#[case::gpt5_capability_selection(capabilities::gpt5_capability_selection)]
#[case::azure_openai_model_redirect(provider::azure_openai_model_redirect)]
#[case::azure_ai_response_model(response::azure_ai_response_model)]
#[case::foundry_chat_auth(environment::foundry_chat_auth)]
#[case::foundry_required_fields(fields::foundry_required_fields)]
fn chat_cases(#[case] check: fn()) {
    check();
}
