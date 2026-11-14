use std::sync::Arc;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::support;
use crate::langfuse_prompt_management::LangfusePromptManagementError;
use crate::langfuse_prompt_management::clients::{LangfusePromptClients, PromptClientRequest};

#[tokio::test]
async fn a_dynamic_host_keeps_environment_credentials_out() {
    let builder = support::RecordingBuilder::default();
    let variables = support::env("http://environment.test");
    let services = support::services(&support::borrowed(&variables), Arc::new(builder.clone()));
    let clients = LangfusePromptClients::default();
    let params = support::dynamic_params(&[
        ("langfuse_public_key", "pk-dynamic"),
        ("langfuse_host", "http://dynamic.test"),
    ]);
    let request = PromptClientRequest::from_dynamic_params(&params).expect("the params parse");
    assert!(!request.allow_env_credentials);
    clients
        .client_init(&services, &request)
        .expect("the client builds");
    let configs = builder.configs();
    assert_eq!(configs.len(), 1);
    assert_eq!(configs[0].public_key.as_deref(), Some("pk-dynamic"));
    assert_eq!(configs[0].secret, None);
    assert_eq!(configs[0].base_url, "http://dynamic.test");
}

#[tokio::test]
async fn a_dynamic_key_without_a_host_mixes_in_environment_credentials() {
    let builder = support::RecordingBuilder::default();
    let variables = support::env("http://environment.test");
    let services = support::services(&support::borrowed(&variables), Arc::new(builder.clone()));
    let clients = LangfusePromptClients::default();
    let params = support::dynamic_params(&[("langfuse_public_key", "pk-dynamic")]);
    let request = PromptClientRequest::from_dynamic_params(&params).expect("the params parse");
    assert!(request.allow_env_credentials);
    clients
        .client_init(&services, &request)
        .expect("the client builds");
    let configs = builder.configs();
    assert_eq!(configs.len(), 1);
    assert_eq!(configs[0].public_key.as_deref(), Some("pk-dynamic"));
    assert_eq!(configs[0].secret.as_deref(), Some(support::SECRET_KEY));
    assert_eq!(configs[0].base_url, "http://environment.test");
}

#[test]
fn a_dynamic_field_of_the_wrong_type_is_named() {
    let params = Json::Object(IndexMap::from([(
        "langfuse_host".to_owned(),
        serde_json::from_str("7").expect("the number parses"),
    )]));
    let error =
        PromptClientRequest::from_dynamic_params(&params).expect_err("a number is not a host");
    assert!(matches!(
        error,
        LangfusePromptManagementError::FieldType {
            field: "langfuse_host"
        }
    ));
    assert_eq!(
        error.to_string(),
        "standard_callback_dynamic_params.langfuse_host must be a string"
    );
}

#[test]
fn dynamic_params_that_are_not_an_object_are_rejected() {
    let error =
        PromptClientRequest::from_dynamic_params(&Json::Null).expect_err("null is not an object");
    assert!(matches!(error, LangfusePromptManagementError::ParamsType));
    assert_eq!(
        error.to_string(),
        "standard_callback_dynamic_params must be an object"
    );
}
