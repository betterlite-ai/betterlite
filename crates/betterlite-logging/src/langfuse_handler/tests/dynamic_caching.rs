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
    let first = LangFuseHandler::logger_for_request(&services, &cache, &per_key("pk-one"), None)
        .await
        .expect("the first logger is created");
    let second = LangFuseHandler::logger_for_request(&services, &cache, &per_key("pk-two"), None)
        .await
        .expect("the second logger is created");
    let again = LangFuseHandler::logger_for_request(&services, &cache, &per_key("pk-one"), None)
        .await
        .expect("the first logger is reused");
    assert!(!Arc::ptr_eq(&first, &second));
    assert!(Arc::ptr_eq(&first, &again));
}
