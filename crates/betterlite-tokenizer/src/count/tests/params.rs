use betterlite_models::catalog::{ModelCatalog, UnknownFields};

use crate::count::messages::message_params;

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
