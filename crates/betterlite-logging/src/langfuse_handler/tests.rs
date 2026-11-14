#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_models::decisions::Json;
use secrecy::SecretString;

use crate::dynamic_logger_cache::DynamicLoggerCache;
use crate::langfuse_handler::SERVICE_NAME;
use crate::langfuse_logger::LangfuseServices;
use crate::langfuse_logger::langfuse::LangFuseLogger;
use crate::langfuse_logger::tests::support as common;

fn dynamic(fields: Vec<(&str, Json)>) -> Json {
    Json::Object(
        fields
            .into_iter()
            .map(|(name, value)| (name.to_owned(), value))
            .collect(),
    )
}

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn services() -> LangfuseServices {
    common::services(&[("LANGFUSE_MOCK", "true")], 50)
}

fn per_key(public_key: &str) -> Json {
    dynamic(vec![
        ("langfuse_public_key", text(public_key)),
        ("langfuse_host", text(common::HOST)),
    ])
}

fn credential_key(public_key: &str) -> BTreeMap<String, SecretString> {
    let mut key = BTreeMap::new();
    key.insert("langfuse_host".to_owned(), common::secret(common::HOST));
    key.insert("langfuse_public_key".to_owned(), common::secret(public_key));
    key
}

fn cached(
    cache: &DynamicLoggerCache<Arc<LangFuseLogger>>,
    key: &BTreeMap<String, SecretString>,
) -> Arc<LangFuseLogger> {
    cache
        .get_cache(key, SERVICE_NAME)
        .expect("the cache reads")
        .expect("the logger is cached")
}

mod logger_selection_tests {
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use crate::langfuse_handler::{LangFuseHandler, SERVICE_NAME};
    use crate::langfuse_logger::langfuse::LangFuseLogger;
    use crate::langfuse_logger::tests::support as common;

    use super::{cached, credential_key, dynamic, per_key, services};

    #[tokio::test]
    async fn an_existing_global_logger_is_returned() {
        let services = services();
        let cache = common::cache();
        let global = Arc::new(
            LangFuseLogger::new(&services, &common::environment_request())
                .await
                .expect("the global logger builds"),
        );
        let logger = LangFuseHandler::logger_for_request(
            &services,
            &cache,
            &dynamic(vec![]),
            Some(Arc::clone(&global)),
        )
        .await
        .expect("the global logger is returned");
        assert!(Arc::ptr_eq(&logger, &global));
        let stored = cache
            .get_cache(&BTreeMap::new(), SERVICE_NAME)
            .expect("the cache reads");
        assert!(stored.is_none());
    }

    #[tokio::test]
    async fn a_global_logger_is_created_and_cached_without_credentials() {
        let services = services();
        let cache = common::cache();
        let logger = LangFuseHandler::logger_for_request(&services, &cache, &dynamic(vec![]), None)
            .await
            .expect("the global logger is created");
        assert_eq!(logger.project_id.as_deref(), Some("mock-project-id"));
        assert!(Arc::ptr_eq(&logger, &cached(&cache, &BTreeMap::new())));
    }

    #[tokio::test]
    async fn a_dynamic_logger_is_cached_under_its_credentials() {
        let services = services();
        let cache = common::cache();
        let params = per_key("pk-selected");
        let logger = LangFuseHandler::logger_for_request(&services, &cache, &params, None)
            .await
            .expect("the dynamic logger is created");
        let again = LangFuseHandler::logger_for_request(&services, &cache, &params, None)
            .await
            .expect("the cached logger is returned");
        assert!(Arc::ptr_eq(&logger, &again));
        assert!(Arc::ptr_eq(
            &logger,
            &cached(&cache, &credential_key("pk-selected"))
        ));
    }
}

mod dynamic_caching {
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use crate::langfuse_handler::{LangFuseHandler, SERVICE_NAME};
    use crate::langfuse_logger::tests::support as common;

    use super::{cached, credential_key, dynamic, per_key, services};

    #[tokio::test]
    async fn global_and_dynamic_loggers_stay_apart() {
        let services = services();
        let cache = common::cache();
        let global = LangFuseHandler::logger_for_request(&services, &cache, &dynamic(vec![]), None)
            .await
            .expect("the global logger is created");
        let per_key_logger =
            LangFuseHandler::logger_for_request(&services, &cache, &per_key("pk-apart"), None)
                .await
                .expect("the dynamic logger is created");
        assert!(!Arc::ptr_eq(&global, &per_key_logger));
        let stored = cache
            .get_cache(&BTreeMap::new(), SERVICE_NAME)
            .expect("the cache reads");
        assert!(stored.is_some());
        assert!(Arc::ptr_eq(
            &per_key_logger,
            &cached(&cache, &credential_key("pk-apart"))
        ));
    }

    #[tokio::test]
    async fn distinct_credentials_get_distinct_loggers() {
        let services = services();
        let cache = common::cache();
        let first =
            LangFuseHandler::logger_for_request(&services, &cache, &per_key("pk-one"), None)
                .await
                .expect("the first logger is created");
        let second =
            LangFuseHandler::logger_for_request(&services, &cache, &per_key("pk-two"), None)
                .await
                .expect("the second logger is created");
        let again =
            LangFuseHandler::logger_for_request(&services, &cache, &per_key("pk-one"), None)
                .await
                .expect("the first logger is reused");
        assert!(!Arc::ptr_eq(&first, &second));
        assert!(Arc::ptr_eq(&first, &again));
    }
}

mod environment_deduplication {
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use rstest::rstest;

    use crate::langfuse_handler::{LangFuseHandler, SERVICE_NAME};
    use crate::langfuse_logger::tests::support as common;

    use super::{dynamic, text};

    #[rstest]
    #[case::empty("")]
    #[case::whitespace("   ")]
    fn empty_or_whitespace_environments_are_not_meaningful(#[case] value: &str) {
        let settings = common::settings(&[]);
        let params = dynamic(vec![("langfuse_environment", text(value))]);
        assert_eq!(
            LangFuseHandler::meaningful_dynamic_environment(&settings, &params)
                .expect("the environment resolves"),
            None
        );
    }

    #[test]
    fn an_environment_matching_the_deployment_variable_is_dropped() {
        let settings = common::settings(&[("LANGFUSE_TRACING_ENVIRONMENT", "production")]);
        let params = dynamic(vec![("langfuse_environment", text("production"))]);
        assert_eq!(
            LangFuseHandler::meaningful_dynamic_environment(&settings, &params)
                .expect("the environment resolves"),
            None
        );
    }

    #[test]
    fn an_environment_matching_the_invalid_fallback_is_dropped() {
        let settings = common::settings(&[("LANGFUSE_TRACING_ENVIRONMENT", "Invalid Env")]);
        let params = dynamic(vec![("langfuse_environment", text("default"))]);
        assert_eq!(
            LangFuseHandler::meaningful_dynamic_environment(&settings, &params)
                .expect("the environment resolves"),
            None
        );
    }

    #[test]
    fn a_different_environment_survives_trimmed() {
        let settings = common::settings(&[("LANGFUSE_TRACING_ENVIRONMENT", "production")]);
        let params = dynamic(vec![("langfuse_environment", text(" staging "))]);
        assert_eq!(
            LangFuseHandler::meaningful_dynamic_environment(&settings, &params)
                .expect("the environment resolves"),
            Some("staging".to_owned())
        );
    }

    #[tokio::test]
    async fn a_matching_environment_does_not_mint_a_logger() {
        let services = common::services(
            &[
                ("LANGFUSE_MOCK", "true"),
                ("LANGFUSE_TRACING_ENVIRONMENT", "production"),
            ],
            50,
        );
        let cache = common::cache();
        let global = LangFuseHandler::logger_for_request(&services, &cache, &dynamic(vec![]), None)
            .await
            .expect("the global logger is created");
        let params = dynamic(vec![("langfuse_environment", text("production"))]);
        let logger = LangFuseHandler::logger_for_request(&services, &cache, &params, None)
            .await
            .expect("the global logger is reused");
        assert!(Arc::ptr_eq(&logger, &global));
        let stored = cache
            .get_cache(&BTreeMap::new(), SERVICE_NAME)
            .expect("the cache reads");
        assert!(stored.is_some());
    }
}

mod credential_detection {
    use betterlite_models::decisions::Json;
    use rstest::rstest;

    use crate::langfuse_handler::{LangFuseHandler, LangfuseSelectionError};
    use crate::langfuse_logger::tests::support as common;

    use super::{dynamic, text};

    #[rstest]
    #[case::host("langfuse_host")]
    #[case::public_key("langfuse_public_key")]
    #[case::secret("langfuse_secret")]
    #[case::secret_key("langfuse_secret_key")]
    fn any_present_field_counts(#[case] field: &str) {
        let settings = common::settings(&[]);
        let params = dynamic(vec![(field, text("value"))]);
        assert!(
            LangFuseHandler::has_dynamic_credentials(&settings, &params).expect("the fields read")
        );
    }

    #[test]
    fn absent_or_null_fields_do_not_count() {
        let settings = common::settings(&[]);
        let nulls = dynamic(vec![
            ("langfuse_host", Json::Null),
            ("langfuse_secret", Json::Null),
        ]);
        assert!(
            !LangFuseHandler::has_dynamic_credentials(&settings, &nulls).expect("the fields read")
        );
        assert!(
            !LangFuseHandler::has_dynamic_credentials(&settings, &dynamic(vec![]))
                .expect("the fields read")
        );
    }

    #[test]
    fn a_meaningful_environment_counts() {
        let settings = common::settings(&[]);
        let params = dynamic(vec![("langfuse_environment", text("staging"))]);
        assert!(
            LangFuseHandler::has_dynamic_credentials(&settings, &params).expect("the fields read")
        );
    }

    #[test]
    fn params_that_are_not_an_object_are_refused() {
        let settings = common::settings(&[]);
        let error = LangFuseHandler::has_dynamic_credentials(&settings, &text("x"))
            .err()
            .expect("the params are refused");
        assert_eq!(
            error.to_string(),
            "standard_callback_dynamic_params must be an object"
        );
        assert!(matches!(error, LangfuseSelectionError::ParamsType));
    }
}

mod config_extraction {
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
        let config = LangFuseHandler::dynamic_logging_config(&settings, &both)
            .expect("the config is extracted");
        assert_eq!(
            config
                .langfuse_secret
                .as_ref()
                .map(ExposeSecret::expose_secret),
            Some("sk-main")
        );
        let empty = dynamic(vec![
            ("langfuse_secret", text("")),
            ("langfuse_secret_key", text("sk-fallback")),
        ]);
        let config = LangFuseHandler::dynamic_logging_config(&settings, &empty)
            .expect("the config is extracted");
        assert_eq!(
            config
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
            .err()
            .expect("the field is refused");
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
}
