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
        Family::Chat => chat::check().await,
        Family::Embeddings => {
            embeddings::check().await;
            embeddings::check_configured_format().await;
        }
        Family::ExplicitParamBeforeConfig => {
            explicit_param_before_config::check_explicit_wins().await;
            explicit_param_before_config::check_globals_chain().await;
            explicit_param_before_config::check_header_resolution();
            explicit_param_before_config::check_defaults_fill_absent_keys_only().await;
        }
        Family::ExperimentalHandler => {
            experimental_handler::check().await;
            experimental_handler::check_empty_extra_map_overwrites().await;
        }
        Family::PreviewMetadata => {
            preview_metadata::check().await;
            preview_metadata::check_limits_to_sixteen().await;
        }
        Family::Http => {
            http_dispatch::check().await;
            http_dispatch::check_failure().await;
            http_dispatch::check_stream();
        }
        Family::TextDefaults => {
            text_defaults::check_plan();
            text_defaults::check_transport_attempt().await;
            text_defaults::check_completion_post_call();
        }
        Family::TextChatProjection => {
            text_chat_projection::check_projection_predicate();
            text_chat_projection::check_log_predicate();
            text_chat_projection::check().await;
            text_chat_projection::check_async_logging().await;
            text_chat_projection::check_text_completion_marker().await;
        }
        Family::CallFailureLogging => {
            call_failure_logging::check().await;
            call_failure_logging::check_experimental().await;
            call_failure_logging::check_preparation_failure_has_no_log().await;
        }
    }
}
