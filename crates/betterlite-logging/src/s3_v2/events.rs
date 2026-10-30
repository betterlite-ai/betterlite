use chrono::NaiveDateTime;
use serde_json::{Map, Value};

use crate::s3_transport::batch_element::{ContentType, S3BatchLoggingElement};
use crate::s3_transport::naming::{object_download_filename, object_key};
use crate::s3_transport::options::{
    PartitionGranularity, resolve_log_prompts_only, resolve_partition_granularity,
};
use crate::s3_v2::{S3Parameters, metadata_text};

#[must_use]
pub fn create_batch_element(
    parameters: &S3Parameters,
    start_time: NaiveDateTime,
    payload: Option<&Map<String, Value>>,
    cold_storage_enabled: bool,
    environment: &dyn betterlite_config::loader::EnvironmentLookup,
) -> Option<S3BatchLoggingElement<Value>> {
    let payload = payload?.clone();
    let id = payload.get("id").and_then(Value::as_str)?.to_owned();
    let mut payload = Value::Object(payload);
    if parameters.strip_base64_files {
        strip_base64(&mut payload);
    }
    let object = payload.as_object()?;
    let prefix = prefix_path(parameters, object);
    let file_name = format!("time-{}_{}", start_time.format("%H-%M-%S-%6f"), id);
    let day_key = object_key(
        &parameters.path,
        &prefix,
        start_time,
        &file_name,
        PartitionGranularity::Day,
    );
    let hour_key = object_key(
        &parameters.path,
        &prefix,
        start_time,
        &file_name,
        PartitionGranularity::Hour,
    );
    let cold_key = cold_storage_enabled
        .then(|| metadata_text(object, "cold_storage_object_key"))
        .flatten();
    let key = cold_key
        .filter(|candidate| *candidate == day_key || *candidate == hour_key)
        .unwrap_or_else(|| {
            object_key(
                &parameters.path,
                &prefix,
                start_time,
                &file_name,
                resolve_partition_granularity(Some(&parameters.partition_granularity), environment),
            )
        });
    let output = if resolve_log_prompts_only(Some(&parameters.log_prompts_only), environment) {
        let mut prompt_only = object.clone();
        prompt_only.insert("response".to_owned(), Value::Null);
        Value::Object(prompt_only)
    } else {
        payload
    };
    Some(S3BatchLoggingElement {
        payload: output,
        s3_object_key: key,
        s3_object_download_filename: object_download_filename(start_time, &id),
        body: None,
        content_type: ContentType::Json,
        retrying_since: None,
    })
}

fn prefix_path(parameters: &S3Parameters, object: &Map<String, Value>) -> String {
    let mut prefixes = Vec::new();
    if parameters.use_team_prefix {
        if let Some(team) = metadata_text(object, "user_api_key_team_alias") {
            prefixes.push(team);
        }
    }
    if parameters.use_key_prefix {
        if let Some(key) = metadata_text(object, "user_api_key_alias") {
            prefixes.push(key);
        }
    }
    if prefixes.is_empty() {
        String::new()
    } else {
        format!("{}/", prefixes.join("/"))
    }
}

fn strip_base64(value: &mut Value) {
    match value {
        Value::Object(map) => {
            map.retain(|key, value| {
                key != "file"
                    && !(key == "type" && value.as_str().is_some_and(|kind| kind != "text"))
            });
            for nested in map.values_mut() {
                strip_base64(nested);
            }
        }
        Value::Array(values) => {
            for nested in values {
                strip_base64(nested);
            }
        }
        Value::String(text) if text.starts_with("data:") && text.contains(";base64,") => {
            *text = "[BASE64_REDACTED]".to_owned()
        }
        _ => {}
    }
}
