#![cfg(test)]

mod chat_create_prefix_and_reasoning;
mod create_affinity_and_interception_order;
mod create_logging_metadata_precedence;
mod mock_response_literals;
mod native_create_options_and_ids;
mod support;

#[tokio::test]
async fn native_create_options_and_ids() {
    native_create_options_and_ids::options_and_ids().await;
    native_create_options_and_ids::encrypted_item_ids().await;
}

#[tokio::test]
async fn chat_create_prefix_and_reasoning() {
    chat_create_prefix_and_reasoning::bridge_selection().await;
    chat_create_prefix_and_reasoning::reasoning().await;
    chat_create_prefix_and_reasoning::timeouts().await;
    chat_create_prefix_and_reasoning::allowed_and_dropped().await;
}

#[tokio::test]
async fn create_affinity_and_interception_order() {
    create_affinity_and_interception_order::affinity_headers().await;
    create_affinity_and_interception_order::affinity_control_characters().await;
    create_affinity_and_interception_order::mcp_first_and_mapped().await;
    create_affinity_and_interception_order::file_search_extras().await;
    create_affinity_and_interception_order::encrypted_support().await;
}

#[tokio::test]
async fn create_logging_metadata_precedence() {
    create_logging_metadata_precedence::logged_parameters().await;
    create_logging_metadata_precedence::metadata_precedence().await;
    create_logging_metadata_precedence::residency_and_client().await;
    create_logging_metadata_precedence::stamped_and_synthetic().await;
    create_logging_metadata_precedence::provider_failure().await;
}

#[tokio::test]
async fn mock_response_literals() {
    mock_response_literals::literal_and_default().await;
    mock_response_literals::shortcut_and_skips().await;
    mock_response_literals::predicate().await;
}
