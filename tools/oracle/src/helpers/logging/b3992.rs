use std::collections::BTreeMap;

use betterlite_logging::s3_transport::option_value::OptionValue;
use betterlite_logging::s3_transport::options::PartitionGranularity;
use betterlite_logging::s3_v2::events::{Event, create_batch_element};
use betterlite_logging::s3_v2::parameters::resolve_s3_parameters;
use betterlite_logging::s3_v2::source::Source;
use betterlite_logging::s3_v2::{Feature, PartitionGranularityCache, S3Parameters};
use chrono::NaiveDateTime;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperError, HelperResult};

mod tests;

pub(crate) struct Op;

#[derive(Deserialize)]
pub(crate) struct Input {
    #[serde(flatten)]
    operation: Operation,
    #[serde(default)]
    settings: BTreeMap<String, String>,
}

#[derive(Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub(crate) enum Operation {
    Params {
        #[serde(default)]
        constructor: Box<Source<Value>>,
        #[serde(default)]
        global: Box<Source<Value>>,
        audit: Option<Box<Source<Value>>>,
    },
    Partition {
        values: Vec<Option<OptionValue>>,
    },
    Event {
        #[serde(default)]
        source: Box<Source<Value>>,
        start_time: NaiveDateTime,
        payload: Option<Value>,
        #[serde(default)]
        cold_storage: bool,
    },
}

impl Helper for Op {
    type Input = Input;
    fn run(input: &Self::Input) -> HelperResult {
        let environment = &input.settings;
        match &input.operation {
            Operation::Params {
                constructor,
                global,
                audit,
            } => {
                let resolved =
                    resolve_s3_parameters(constructor, global, audit.as_deref(), environment);
                Ok(summary(&resolved))
            }
            Operation::Partition { values } => Ok(partitions(values, environment)),
            Operation::Event {
                source,
                start_time,
                payload,
                cold_storage,
            } => event(
                source,
                *start_time,
                payload.as_ref(),
                *cold_storage,
                environment,
            ),
        }
    }
}

fn event(
    source: &Source<Value>,
    start_time: NaiveDateTime,
    payload: Option<&Value>,
    cold_storage: bool,
    environment: &BTreeMap<String, String>,
) -> HelperResult {
    let parameters = resolve_s3_parameters(&Source::default(), source, None, environment);
    let element = create_batch_element(
        &parameters,
        &Event {
            start_time,
            payload,
            cold_storage_enabled: cold_storage,
        },
        &mut PartitionGranularityCache::default(),
        environment,
    )
    .map_err(|error| {
        Box::new(HelperError {
            class: "S3PayloadError".to_owned(),
            message: error.to_string(),
            status: None,
            error_type: None,
            param: None,
            code: None,
        })
    })?;
    Ok(json!({"element":element}))
}

fn summary(parameters: &S3Parameters<Value>) -> Value {
    let connection = &parameters.connection;
    json!({
        "bucket_name": connection.bucket_name,
        "region_name": connection.region_name,
        "api_version": connection.api_version,
        "endpoint_url": connection.endpoint_url,
        "path": connection.path,
        "config": connection.config,
        "max_concurrent_uploads": parameters.max_concurrent_uploads,
        "max_queue_size": parameters.max_queue_size,
        "max_retry_age_seconds": parameters.max_retry_age_seconds.map(|age| age.as_secs()),
        "max_adaptive_concurrency": parameters.max_adaptive_concurrency,
        "use_ssl": parameters.features.contains(&Feature::UseSsl),
        "use_team_prefix": parameters.features.contains(&Feature::TeamPrefix),
        "use_key_prefix": parameters.features.contains(&Feature::KeyPrefix),
        "strip_base64_files": parameters.features.contains(&Feature::StripBase64),
        "use_virtual_hosted_style": parameters.features.contains(&Feature::VirtualHostedStyle),
        "drop_on_terminal_error": parameters.features.contains(&Feature::DropOnTerminalError),
        "adaptive_concurrency": parameters.features.contains(&Feature::AdaptiveConcurrency),
        "batch_file_upload": parameters.features.contains(&Feature::BatchFileUpload),
        "server_side_encryption": parameters.encryption.as_ref().map(|encryption| &encryption.algorithm),
        "sse_kms_key_id": parameters.encryption.as_ref().and_then(|encryption| encryption.kms_key_id.as_ref()),
    })
}

fn partitions(values: &[Option<OptionValue>], environment: &BTreeMap<String, String>) -> Value {
    let mut cache = PartitionGranularityCache::default();
    json!(
        values
            .iter()
            .map(|value| match cache.resolve(value.as_ref(), environment) {
                PartitionGranularity::Day => "day",
                PartitionGranularity::Hour => "hour",
            })
            .collect::<Vec<_>>()
    )
}
