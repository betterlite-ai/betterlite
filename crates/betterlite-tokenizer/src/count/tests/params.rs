use std::sync::Arc;

use betterlite_models::catalog::{ModelCatalog, UnknownFields};

use crate::count::messages::message_params;
use crate::count::settings::Settings;

use super::support;

const CATALOG: &[u8] = br#"{
  "gpt-3.5-turbo-0301": {"litellm_provider": "openai"},
  "gpt-3.5-turbo": {"litellm_provider": "openai"},
  "some-cohere-model": {"litellm_provider": "cohere"}
}"#;

#[test]
fn legacy_accounting_uses_the_old_constants() {
    let catalog = ModelCatalog::parse(CATALOG, UnknownFields::Reject).unwrap();
    let params = message_params("gpt-3.5-turbo-0301", &catalog);
    assert_eq!(params.tokens_per_message, 4);
    assert_eq!(params.tokens_per_name, -1);
}

#[test]
fn every_other_model_uses_three_and_one() {
    let catalog = ModelCatalog::parse(CATALOG, UnknownFields::Reject).unwrap();
    assert_eq!(
        message_params("gpt-3.5-turbo", &catalog).tokens_per_message,
        3
    );
    assert_eq!(
        message_params("some-cohere-model", &catalog).tokens_per_message,
        3
    );
    assert_eq!(message_params("unknown-model", &catalog).tokens_per_name, 1);
}

#[tokio::test]
async fn legacy_accounting_counts_a_named_message() {
    let directory = tempfile::tempdir().unwrap();
    let catalog = Arc::new(ModelCatalog::parse(CATALOG, UnknownFields::Reject).unwrap());
    let counter = support::counter_with_catalog(Settings::default(), directory.path(), catalog);
    let request = r#"{"model":"gpt-3.5-turbo-0301","messages":[{"role":"user","content":"hello world","name":"bob"}]}"#;
    // 4 per message + role(1) + content(2) + name(1) - 1 for the name + 3 reply priming
    assert_eq!(support::count_request(&counter, request).await.unwrap(), 10);
}
