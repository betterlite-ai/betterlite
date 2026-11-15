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

mod logger_selection_tests;

mod dynamic_caching;

mod environment_deduplication;

mod credential_detection;

mod config_extraction;
