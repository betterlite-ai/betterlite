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
fn an_environment_matching_an_invalid_deployment_value_is_dropped() {
    let settings = common::settings(&[("LANGFUSE_TRACING_ENVIRONMENT", "Invalid Env")]);
    let params = dynamic(vec![("langfuse_environment", text("Invalid Env"))]);
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
