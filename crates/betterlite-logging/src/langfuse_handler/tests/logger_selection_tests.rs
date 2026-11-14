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
