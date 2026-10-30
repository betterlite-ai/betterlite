use betterlite_config::loader::EnvironmentLookup;
use serde_json::{Map, Value};
use std::time::Duration;

use crate::s3_transport::option_value::OptionValue;
use crate::s3_transport::options::{
    Bound, PartitionGranularity, Toggle, default_adaptive_limit, resolve_bound,
    resolve_log_prompts_only, resolve_max_retry_age, resolve_partition_granularity, resolve_toggle,
};

mod events;
mod tests;

pub use events::create_batch_element;

const LOG_PROMPTS_ONLY_VARIABLE: &str = "S3_LOG_PROMPTS_ONLY";
const PARTITION_GRANULARITY_VARIABLE: &str = "S3_PARTITION_GRANULARITY";
const DEFAULT_MAX_CONCURRENT_UPLOADS: usize = 10;
const DEFAULT_MAX_QUEUE_SIZE: usize = 50_000;

#[derive(Clone, Debug, PartialEq)]
pub struct S3Parameters {
    pub bucket_name: Option<String>,
    pub region_name: Option<String>,
    pub api_version: Option<String>,
    pub use_ssl: bool,
    pub verify: Option<bool>,
    pub endpoint_url: Option<String>,
    pub aws_access_key_id: Option<String>,
    pub aws_secret_access_key: Option<String>,
    pub aws_session_token: Option<String>,
    pub aws_session_name: Option<String>,
    pub aws_profile_name: Option<String>,
    pub aws_role_name: Option<String>,
    pub aws_web_identity_token: Option<String>,
    pub aws_sts_endpoint: Option<String>,
    pub config: Option<Value>,
    pub path: String,
    pub use_team_prefix: bool,
    pub strip_base64_files: bool,
    pub use_key_prefix: bool,
    pub use_virtual_hosted_style: bool,
    pub server_side_encryption: Option<String>,
    pub sse_kms_key_id: Option<String>,
    pub log_prompts_only: OptionValue,
    pub partition_granularity: OptionValue,
    pub max_concurrent_uploads: usize,
    pub max_queue_size: usize,
    pub max_retry_age_seconds: Option<Duration>,
    pub drop_on_terminal_error: bool,
    pub adaptive_concurrency: bool,
    pub max_adaptive_concurrency: usize,
    pub batch_file_upload: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct S3ConstructorParameters {
    pub bucket_name: Option<String>,
    pub region_name: Option<String>,
    pub api_version: Option<String>,
    pub use_ssl: bool,
    pub verify: Option<bool>,
    pub endpoint_url: Option<String>,
    pub path: Option<String>,
    pub max_concurrent_uploads: usize,
    pub max_queue_size: Option<usize>,
    pub max_retry_age_seconds: Option<Duration>,
}

impl Default for S3ConstructorParameters {
    fn default() -> Self {
        Self {
            bucket_name: None,
            region_name: None,
            api_version: None,
            use_ssl: true,
            verify: None,
            endpoint_url: None,
            path: None,
            max_concurrent_uploads: DEFAULT_MAX_CONCURRENT_UPLOADS,
            max_queue_size: None,
            max_retry_age_seconds: Some(Duration::from_secs(3_600)),
        }
    }
}

fn option_value(source: &Map<String, Value>, key: &str) -> Option<OptionValue> {
    let value = source
        .get(key)
        .and_then(|value| serde_json::from_value(value.clone()).ok());
    value
}
fn text(source: &Map<String, Value>, key: &str) -> Option<String> {
    let value = source.get(key).and_then(Value::as_str).map(str::to_owned);
    value
}
fn bool_value(source: &Map<String, Value>, key: &str, fallback: bool) -> bool {
    let value = option_value(source, key)
        .and_then(|value| value.as_bool())
        .unwrap_or(fallback);
    value
}
#[must_use]
pub fn resolve_s3_parameters(
    constructor: &S3ConstructorParameters,
    global_source: &Map<String, Value>,
    audit_source: Option<&Map<String, Value>>,
    environment: &dyn EnvironmentLookup,
) -> S3Parameters {
    let source = audit_source.unwrap_or(global_source);
    let configured = |key: &str| option_value(source, key);
    let queue_fallback = constructor.max_queue_size.unwrap_or(DEFAULT_MAX_QUEUE_SIZE);
    let retry = configured("s3_max_retry_age_seconds");
    let retry_age = retry
        .as_ref()
        .map_or(constructor.max_retry_age_seconds, |value| match value {
            OptionValue::Text(text) if text.is_empty() => constructor.max_retry_age_seconds,
            _ => resolve_max_retry_age(Some(value), constructor.max_retry_age_seconds),
        });
    let log_prompts_only = configured("s3_log_prompts_only").unwrap_or_else(|| {
        OptionValue::Text(
            environment
                .lookup(LOG_PROMPTS_ONLY_VARIABLE)
                .unwrap_or_default(),
        )
    });
    let partition = configured("s3_partition_granularity").unwrap_or_else(|| {
        OptionValue::Text(
            environment
                .lookup(PARTITION_GRANULARITY_VARIABLE)
                .unwrap_or_default(),
        )
    });
    let concurrent = configured("s3_max_concurrent_uploads");
    let adaptive = configured("s3_max_adaptive_concurrency");
    S3Parameters {
        bucket_name: text(source, "s3_bucket_name")
            .filter(|value| !value.is_empty())
            .or(constructor.bucket_name.clone()),
        region_name: text(source, "s3_region_name")
            .filter(|value| !value.is_empty())
            .or(constructor.region_name.clone()),
        api_version: text(source, "s3_api_version")
            .filter(|value| !value.is_empty())
            .or(constructor.api_version.clone()),
        use_ssl: bool_value(source, "s3_use_ssl", constructor.use_ssl),
        verify: source
            .get("s3_verify")
            .and_then(Value::as_bool)
            .or(constructor.verify),
        endpoint_url: text(source, "s3_endpoint_url")
            .filter(|value| !value.is_empty())
            .or(constructor.endpoint_url.clone()),
        aws_access_key_id: text(source, "s3_aws_access_key_id"),
        aws_secret_access_key: text(source, "s3_aws_secret_access_key"),
        aws_session_token: text(source, "s3_aws_session_token"),
        aws_session_name: text(source, "s3_aws_session_name"),
        aws_profile_name: text(source, "s3_aws_profile_name"),
        aws_role_name: text(source, "s3_aws_role_name"),
        aws_web_identity_token: text(source, "s3_aws_web_identity_token"),
        aws_sts_endpoint: text(source, "s3_aws_sts_endpoint"),
        config: source.get("s3_config").cloned(),
        path: text(source, "s3_path")
            .filter(|value| !value.is_empty())
            .or(constructor.path.clone())
            .unwrap_or_default(),
        use_team_prefix: bool_value(source, "s3_use_team_prefix", false),
        strip_base64_files: bool_value(source, "s3_strip_base64_files", false),
        use_key_prefix: bool_value(source, "s3_use_key_prefix", false),
        use_virtual_hosted_style: bool_value(source, "s3_use_virtual_hosted_style", false),
        server_side_encryption: text(source, "s3_server_side_encryption"),
        sse_kms_key_id: text(source, "s3_sse_kms_key_id"),
        log_prompts_only: log_prompts_only.clone(),
        partition_granularity: partition,
        max_concurrent_uploads: resolve_bound(
            Bound::MaxConcurrentUploads,
            concurrent.as_ref(),
            constructor.max_concurrent_uploads,
        ),
        max_queue_size: resolve_bound(
            Bound::MaxQueueSize,
            configured("s3_max_queue_size").as_ref(),
            queue_fallback,
        ),
        max_retry_age_seconds: retry_age,
        drop_on_terminal_error: resolve_toggle(
            Toggle::DropOnTerminalError,
            configured("s3_drop_on_terminal_error").as_ref(),
        ),
        adaptive_concurrency: resolve_toggle(
            Toggle::AdaptiveConcurrency,
            configured("s3_adaptive_concurrency").as_ref(),
        ),
        max_adaptive_concurrency: resolve_bound(
            Bound::MaxAdaptiveConcurrency,
            adaptive.as_ref(),
            default_adaptive_limit(environment),
        ),
        batch_file_upload: resolve_toggle(
            Toggle::BatchFileUpload,
            configured("s3_batch_file_upload").as_ref(),
        ),
    }
}

#[derive(Debug, Default)]
pub struct PartitionGranularityCache {
    raw: Option<OptionValue>,
    resolved: Option<PartitionGranularity>,
}

impl PartitionGranularityCache {
    #[must_use]
    pub fn resolve(
        &mut self,
        configured: Option<&OptionValue>,
        environment: &dyn EnvironmentLookup,
    ) -> PartitionGranularity {
        let raw = configured.cloned().or_else(|| {
            environment
                .lookup(PARTITION_GRANULARITY_VARIABLE)
                .map(OptionValue::Text)
        });
        if self.raw == raw {
            if let Some(resolved) = self.resolved {
                return resolved;
            }
        }
        let resolved = resolve_partition_granularity(raw.as_ref(), environment);
        self.raw = raw;
        self.resolved = Some(resolved);
        resolved
    }
}

pub(crate) fn metadata_text(object: &Map<String, Value>, key: &str) -> Option<String> {
    let value = object
        .get("metadata")
        .and_then(Value::as_object)
        .and_then(|metadata| metadata.get(key))
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    value
}
