use std::collections::BTreeMap;
use std::fmt::Display;
use std::future::Future;

use async_trait::async_trait;
use betterlite_server::bucket_config::error::BucketConfigError;
use betterlite_server::bucket_config::include_key::resolve_include_object_key;
use betterlite_server::bucket_config::includes::load_config_from_bucket;
use betterlite_server::bucket_config::object::BucketObjectReader;
use betterlite_server::bucket_config::source::{BucketConfigSource, BucketType};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperError, HelperResult};

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
pub(crate) struct Input {
    pub operation: String,
    #[serde(default)]
    pub config_object_key: String,
    #[serde(default)]
    pub include_entry: String,
    #[serde(default)]
    pub object_key: String,
    #[serde(default)]
    pub objects: BTreeMap<String, Value>,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        block_on(run(input))?
    }
}

fn block_on<F>(future: F) -> Result<F::Output, Box<HelperError>>
where
    F: Future,
{
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .start_paused(true)
        .build()
        .map(|runtime| runtime.block_on(future))
        .map_err(|error| helper_error("ValueError", error.to_string()))
}

async fn run(input: &Input) -> HelperResult {
    match input.operation.as_str() {
        "include_key" => Ok(json!({
            "key": resolve_include_object_key(&input.config_object_key, &input.include_entry),
        })),
        "load_config" => load_config(input).await,
        other => Err(helper_error(
            "ValueError",
            format!("unsupported operation {other}"),
        )),
    }
}

async fn load_config(input: &Input) -> HelperResult {
    let reader = JsonReader {
        objects: input.objects.clone(),
    };
    let source = BucketConfigSource {
        bucket_type: BucketType::S3,
        bucket_name: "bucket".to_owned(),
        object_key: input.object_key.clone(),
    };
    match load_config_from_bucket(&source, &reader).await {
        Ok(Some(config)) => Ok(json!({"config": config})),
        Ok(None) => Ok(json!({"config": Value::Null})),
        Err(error) => Err(bucket_error(&error)),
    }
}

fn bucket_error(error: &BucketConfigError) -> Box<HelperError> {
    if matches!(error, BucketConfigError::IncludedUnreadable { .. }) {
        return helper_error("FileNotFoundError", error.to_string());
    }
    helper_error("ValueError", error.to_string())
}

struct JsonReader {
    objects: BTreeMap<String, Value>,
}

#[async_trait]
impl BucketObjectReader for JsonReader {
    async fn read_object(&self, object_key: &str) -> Option<Vec<u8>> {
        self.objects
            .get(object_key)
            .filter(|value| !value.is_null())
            .and_then(|value| serde_json::to_vec(value).ok())
    }
}

fn helper_error<M>(class: &str, message: M) -> Box<HelperError>
where
    M: Display,
{
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_string(),
    })
}
