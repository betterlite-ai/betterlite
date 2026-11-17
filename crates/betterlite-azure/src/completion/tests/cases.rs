#![cfg(test)]

use rstest::rstest;

mod auth;
mod errors;
mod projection;
mod prompts;
mod retries;
mod streams;
mod support;

#[rstest]
#[case::token_prompt(prompts::token_prompt)]
#[case::stream_text_choices(streams::stream_text_choices)]
#[case::completion_errors(errors::completion_errors)]
#[case::azure_text_prompt_every_call(prompts::azure_text_prompt_every_call)]
#[case::cloudflare_model_null(prompts::cloudflare_model_null)]
#[case::max_retries_default_two(retries::max_retries_default_two)]
#[case::invalid_retries_422(retries::invalid_retries_422)]
#[case::missing_model_messages_422(errors::missing_model_messages_422)]
#[case::azure_text_auth(auth::azure_text_auth)]
#[case::azure_text_stream_dispatch(streams::azure_text_stream_dispatch)]
#[case::azure_text_client_errors(errors::azure_text_client_errors)]
#[case::azure_text_response_projection(projection::azure_text_response_projection)]
fn completion_cases(#[case] scenario: fn()) {
    scenario();
}
