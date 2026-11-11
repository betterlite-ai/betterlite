use std::collections::BTreeMap;
use std::time::SystemTime;

use betterlite_models::decisions::Json;
use betterlite_secret_managers::secret_lookup::SecretLookup;
use secrecy::ExposeSecret as _;
use serde_json::value::RawValue;

use super::wire::{
    BedrockCreateRequest, InputDataConfig, OutputDataConfig, S3InputDataConfig, S3OutputDataConfig,
};
use super::{
    COMPLETION_WINDOW_24H, DEFAULT_PREFIX, ENCRYPTION_KEY_PARAM, JOB_ID_LENGTH, OUTPUT_BUCKET_ENV,
    OUTPUT_BUCKET_PARAM, OWNER_PARAM, ROLE_ARN_ENV, ROLE_ARN_PARAM, TAGS_PARAM, TIMEOUT_HOURS,
};
use crate::batch_create::BatchCreateError;
use crate::batch_create::tags::validate_bedrock_tags;
use crate::s3_access::settings::{
    S3Setting, S3SettingSources, SettingMap, SettingValue, resolve_s3_setting,
};
use crate::s3_access::uri::parse_s3_uri;
use crate::signing::target::RegionInputs;

pub struct CreateBatchInput<'text> {
    pub model: &'text str,
    pub input_file_id: Option<&'text str>,
    pub completion_window: Option<&'text str>,
    pub deployment_params: &'text BTreeMap<String, Box<RawValue>>,
    pub request_params: &'text BTreeMap<String, Box<RawValue>>,
    pub environment: &'text BTreeMap<String, String>,
    pub secrets: &'text SecretLookup,
    pub job_prefix: Option<&'text str>,
    pub unique_id: &'text str,
    pub region_inputs: RegionInputs<'text>,
    pub now: SystemTime,
}

struct OwnedSettings {
    owner: Option<String>,
    encryption_key_id: Option<String>,
}

pub fn build_create_request(
    input: &CreateBatchInput<'_>,
) -> Result<BedrockCreateRequest, BatchCreateError> {
    let file_id = input
        .input_file_id
        .filter(|value| !value.is_empty())
        .ok_or(BatchCreateError::InputFileIdRequired)?;
    let (input_bucket, input_key) = parse_s3_uri(file_id)?;
    let output_bucket = output_bucket(input, &input_bucket)?;
    let role_arn = role_arn(input)?;
    if input.model.is_empty() {
        return Err(BatchCreateError::ModelRequired);
    }
    let prefix = input.job_prefix.unwrap_or(DEFAULT_PREFIX);
    let job_name = format!(
        "{prefix}-batch-{}",
        input
            .unique_id
            .chars()
            .take(JOB_ID_LENGTH)
            .collect::<String>()
    );
    let owned = owner_and_encryption_key(input)?;
    let tags = tag_source(input)?
        .map(|raw| validate_bedrock_tags(&raw))
        .transpose()?;
    let (input_data_config, output_data_config) =
        data_configs(&input_key, &output_bucket, &job_name, &input_bucket, &owned);
    Ok(BedrockCreateRequest {
        model_id: input.model.to_owned(),
        job_name,
        input_data_config,
        output_data_config,
        role_arn,
        tags,
        timeout_duration_in_hours: input
            .completion_window
            .filter(|window| *window == COMPLETION_WINDOW_24H)
            .map(|_window| TIMEOUT_HOURS),
    })
}

fn data_configs(
    input_key: &str,
    output_bucket: &str,
    job_name: &str,
    input_bucket: &str,
    owned: &OwnedSettings,
) -> (InputDataConfig, OutputDataConfig) {
    let input_uri = format!("s3://{input_bucket}/{input_key}");
    let output_uri = format!("s3://{output_bucket}/litellm-batch-outputs/{job_name}/");
    (
        InputDataConfig {
            s3_input_data_config: S3InputDataConfig {
                s3_uri: input_uri,
                s3_bucket_owner: owned.owner.clone(),
            },
        },
        OutputDataConfig {
            s3_output_data_config: S3OutputDataConfig {
                s3_uri: output_uri,
                s3_bucket_owner: owned.owner.clone(),
                s3_encryption_key_id: owned.encryption_key_id.clone(),
            },
        },
    )
}

fn output_bucket(
    input: &CreateBatchInput<'_>,
    input_bucket: &str,
) -> Result<String, BatchCreateError> {
    let configured = read_string(input.deployment_params, OUTPUT_BUCKET_PARAM)?;
    Ok(configured
        .filter(|bucket| !bucket.is_empty())
        .or_else(|| {
            input
                .environment
                .get(OUTPUT_BUCKET_ENV)
                .filter(|bucket| !bucket.is_empty())
                .cloned()
        })
        .unwrap_or_else(|| input_bucket.to_owned()))
}

fn role_arn(input: &CreateBatchInput<'_>) -> Result<String, BatchCreateError> {
    let deployment = read_string(input.deployment_params, ROLE_ARN_PARAM)?;
    let request = read_string(input.request_params, ROLE_ARN_PARAM)?;
    deployment
        .filter(|role| !role.is_empty())
        .or_else(|| request.filter(|role| !role.is_empty()))
        .or_else(|| {
            input
                .environment
                .get(ROLE_ARN_ENV)
                .filter(|role| !role.is_empty())
                .cloned()
        })
        .ok_or(BatchCreateError::RoleArnRequired)
}

fn tag_source(input: &CreateBatchInput<'_>) -> Result<Option<Json>, BatchCreateError> {
    let configured = read_json(input.deployment_params, TAGS_PARAM)?;
    if let Some(value) = configured.filter(|value| !matches!(value, Json::Null)) {
        return Ok(Some(value));
    }
    Ok(read_json(input.request_params, TAGS_PARAM)?.filter(|value| !matches!(value, Json::Null)))
}

fn owner_and_encryption_key(
    input: &CreateBatchInput<'_>,
) -> Result<OwnedSettings, BatchCreateError> {
    let names = [OWNER_PARAM, ENCRYPTION_KEY_PARAM];
    let deployment = setting_map(input.deployment_params, &names);
    let request = setting_map(input.request_params, &names);
    let sources = S3SettingSources {
        params: &deployment,
        optional_params: &request,
        secrets: input.secrets,
        environment: input.environment,
    };
    Ok(OwnedSettings {
        owner: resolve_s3_setting(S3Setting::BucketOwner, &sources)?
            .map(|value| value.expose_secret().to_owned()),
        encryption_key_id: resolve_s3_setting(S3Setting::EncryptionKeyId, &sources)?
            .map(|value| value.expose_secret().to_owned()),
    })
}

fn setting_map(params: &BTreeMap<String, Box<RawValue>>, names: &[&str]) -> SettingMap {
    let mut map = SettingMap::new();
    for name in names {
        let Some(raw) = params.get(*name) else {
            continue;
        };
        let value = serde_json::from_str::<Json>(raw.get()).unwrap_or(Json::Null);
        let setting = match value {
            Json::Text(text) => SettingValue::Text(text),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
                SettingValue::Other
            }
        };
        map.insert((*name).to_owned(), setting);
    }
    map
}

fn read_string(
    params: &BTreeMap<String, Box<RawValue>>,
    field: &'static str,
) -> Result<Option<String>, BatchCreateError> {
    let Some(raw) = params.get(field) else {
        return Ok(None);
    };
    serde_json::from_str::<Option<String>>(raw.get())
        .map_err(|source| BatchCreateError::FieldType { field, source })
}

fn read_json(
    params: &BTreeMap<String, Box<RawValue>>,
    field: &'static str,
) -> Result<Option<Json>, BatchCreateError> {
    let Some(raw) = params.get(field) else {
        return Ok(None);
    };
    serde_json::from_str::<Json>(raw.get())
        .map(Some)
        .map_err(|source| BatchCreateError::FieldJson { field, source })
}
