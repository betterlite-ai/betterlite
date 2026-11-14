use betterlite_models::decisions::Json;
use secrecy::ExposeSecret;

use crate::langfuse_handler::{LangFuseHandler, LangfuseSelectionError};
use crate::langfuse_logger::tests::support as common;

use super::{dynamic, text};

#[test]
fn all_four_fields_are_extracted() {
    let settings = common::settings(&[]);
    let params = dynamic(vec![
        ("langfuse_secret", text("sk-secret")),
        ("langfuse_public_key", text("pk-public")),
        ("langfuse_host", text("https://langfuse.example")),
        ("langfuse_environment", text("staging")),
    ]);
    let config = LangFuseHandler::dynamic_logging_config(&settings, &params)
        .expect("the config is extracted");
    assert_eq!(
        config
            .langfuse_secret
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some("sk-secret")
    );
    assert_eq!(config.langfuse_public_key.as_deref(), Some("pk-public"));
    assert_eq!(
        config.langfuse_host.as_deref(),
        Some("https://langfuse.example")
    );
    assert_eq!(config.langfuse_environment.as_deref(), Some("staging"));
}

#[test]
fn the_secret_key_field_falls_back_and_the_secret_wins() {
    let settings = common::settings(&[]);
    let fallback = dynamic(vec![("langfuse_secret_key", text("sk-fallback"))]);
    let config = LangFuseHandler::dynamic_logging_config(&settings, &fallback)
        .expect("the config is extracted");
    assert_eq!(
        config
            .langfuse_secret
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some("sk-fallback")
    );
    let both = dynamic(vec![
        ("langfuse_secret", text("sk-main")),
        ("langfuse_secret_key", text("sk-fallback")),
    ]);
    let both_config =
        LangFuseHandler::dynamic_logging_config(&settings, &both).expect("the config is extracted");
    assert_eq!(
        both_config
            .langfuse_secret
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some("sk-main")
    );
    let empty = dynamic(vec![
        ("langfuse_secret", text("")),
        ("langfuse_secret_key", text("sk-fallback")),
    ]);
    let empty_config = LangFuseHandler::dynamic_logging_config(&settings, &empty)
        .expect("the config is extracted");
    assert_eq!(
        empty_config
            .langfuse_secret
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some("sk-fallback")
    );
}

#[test]
fn absent_fields_stay_absent() {
    let settings = common::settings(&[]);
    let config = LangFuseHandler::dynamic_logging_config(&settings, &dynamic(vec![]))
        .expect("the config is extracted");
    assert!(config.langfuse_secret.is_none());
    assert!(config.langfuse_public_key.is_none());
    assert!(config.langfuse_host.is_none());
    assert!(config.langfuse_environment.is_none());
}

#[test]
fn a_non_string_field_is_named() {
    let params = dynamic(vec![("langfuse_public_key", Json::Bool(true))]);
    let error = LangFuseHandler::dynamic_logging_config(&common::settings(&[]), &params)
        .expect_err("the field is refused");
    assert_eq!(
        error.to_string(),
        "standard_callback_dynamic_params.langfuse_public_key must be a string"
    );
    assert!(matches!(
        error,
        LangfuseSelectionError::FieldType {
            field: "langfuse_public_key"
        }
    ));
}
