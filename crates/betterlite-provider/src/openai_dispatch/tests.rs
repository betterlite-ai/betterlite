#![cfg(test)]

mod call_failure_logging;
mod chat;
mod credential_identity_isolation;
mod embeddings;
mod experimental_handler;
mod explicit_param_before_config;
mod http_dispatch;
mod preview_metadata;
mod secret_safe_failure;
mod support;
mod text_chat_projection;
mod text_defaults;
mod unsupported_operation;

use rstest::rstest;

#[derive(Clone, Copy)]
enum Family {
    Chat,
    Embeddings,
    ExplicitParamBeforeConfig,
    ExperimentalHandler,
    PreviewMetadata,
    Http,
    TextDefaults,
    TextChatProjection,
    CallFailureLogging,
}

// One row per openai-provider oracle case file, each driving the production dispatcher.
#[rstest]
#[case::chat_dispatch(Family::Chat)]
#[case::embeddings_dispatch(Family::Embeddings)]
#[case::explicit_param_before_config(Family::ExplicitParamBeforeConfig)]
#[case::experimental_handler(Family::ExperimentalHandler)]
#[case::preview_metadata(Family::PreviewMetadata)]
#[case::openai_provider_dispatch_http(Family::Http)]
#[case::text_defaults(Family::TextDefaults)]
#[case::text_chat_projection(Family::TextChatProjection)]
#[case::call_failure_logging(Family::CallFailureLogging)]
#[tokio::test]
async fn openai_dispatch_cases(#[case] family: Family) {
    match family {
        Family::Chat => chat::check_all().await,
        Family::Embeddings => embeddings::check_all().await,
        Family::ExplicitParamBeforeConfig => explicit_param_before_config::check_all().await,
        Family::ExperimentalHandler => experimental_handler::check_all().await,
        Family::PreviewMetadata => preview_metadata::check_all().await,
        Family::Http => http_dispatch::check_all().await,
        Family::TextDefaults => text_defaults::check_all().await,
        Family::TextChatProjection => text_chat_projection::check_all().await,
        Family::CallFailureLogging => call_failure_logging::check_all().await,
    }
}
