use std::collections::BTreeMap;
use std::fmt::{self, Debug, Formatter};
use std::future::Future;
use std::time::SystemTime;

use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_context::aws_partition::get_aws_partition;
use betterlite_models::decisions::Json;
use betterlite_secret_managers::secret_lookup::SecretLookup;
use indexmap::IndexMap;
use secrecy::ExposeSecret as _;
use serde::Serialize;
use serde_json::value::RawValue;

use super::BatchCreateError;
use super::tags::{BedrockTag, validate_bedrock_tags};
use crate::aws_settings::sources::Credentials;
use crate::credentials::error::CredentialError;
use crate::request_policy::aws_params::merge_aws_request_params;
use crate::request_policy::{Params, RequestValue};
use crate::s3_access::settings::{
    S3Setting, S3SettingSources, SettingMap, SettingValue, resolve_s3_setting,
};
use crate::s3_access::uri::parse_s3_uri;
use crate::signing::batch::{BatchData, BatchRequest, sign_batch_request};
use crate::signing::request::AwsSignedRequest;
use crate::signing::target::{RegionInputs, resolve_region};

const ROLE_ARN_PARAM: &str = "aws_batch_role_arn";
const ROLE_ARN_ENV: &str = "AWS_BATCH_ROLE_ARN";
const OUTPUT_BUCKET_PARAM: &str = "s3_output_bucket_name";
const OUTPUT_BUCKET_ENV: &str = "AWS_S3_OUTPUT_BUCKET_NAME";
const OWNER_PARAM: &str = "s3_bucket_owner";
const ENCRYPTION_KEY_PARAM: &str = "s3_encryption_key_id";
const TAGS_PARAM: &str = "bedrock_tags";
const DEFAULT_PREFIX: &str = "litellm";
const JOB_ID_LENGTH: usize = 8;
const COMPLETION_WINDOW_24H: &str = "24h";
const TIMEOUT_HOURS: i64 = 24;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct S3InputDataConfig {
    #[serde(rename = "s3Uri")]
    pub s3_uri: String,
    #[serde(rename = "s3BucketOwner", skip_serializing_if = "Option::is_none")]
    pub s3_bucket_owner: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct S3OutputDataConfig {
    #[serde(rename = "s3Uri")]
    pub s3_uri: String,
    #[serde(rename = "s3BucketOwner", skip_serializing_if = "Option::is_none")]
    pub s3_bucket_owner: Option<String>,
    #[serde(rename = "s3EncryptionKeyId", skip_serializing_if = "Option::is_none")]
    pub s3_encryption_key_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InputDataConfig {
    #[serde(rename = "s3InputDataConfig")]
    pub s3_input_data_config: S3InputDataConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OutputDataConfig {
    #[serde(rename = "s3OutputDataConfig")]
    pub s3_output_data_config: S3OutputDataConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BedrockCreateRequest {
    #[serde(rename = "modelId")]
    pub model_id: String,
    #[serde(rename = "jobName")]
    pub job_name: String,
    #[serde(rename = "inputDataConfig")]
    pub input_data_config: InputDataConfig,
    #[serde(rename = "outputDataConfig")]
    pub output_data_config: OutputDataConfig,
    #[serde(rename = "roleArn")]
    pub role_arn: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<BedrockTag>>,
    #[serde(
        rename = "timeoutDurationInHours",
        skip_serializing_if = "Option::is_none"
    )]
    pub timeout_duration_in_hours: Option<i64>,
}

#[derive(Clone, PartialEq, Eq)]
pub struct SignedBatchRequest {
    pub method: String,
    pub url: String,
    pub headers: BTreeMap<String, String>,
    pub data: String,
}

// header values and the signed body never reach Debug, since a session token rides in the headers
impl Debug for SignedBatchRequest {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let names: Vec<&str> = self.headers.keys().map(String::as_str).collect();
        f.debug_struct("SignedBatchRequest")
            .field("method", &self.method)
            .field("url", &self.url)
            .field("header_names", &names)
            .field("data_length", &self.data.len())
            .finish()
    }
}

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

pub struct SignedCreateInput<'text> {
    pub model: &'text str,
    pub request: &'text BedrockCreateRequest,
    pub deployment_params: &'text BTreeMap<String, Box<RawValue>>,
    pub request_params: &'text BTreeMap<String, Box<RawValue>>,
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

pub fn complete_batch_url(
    model: &str,
    deployment_params: &BTreeMap<String, Box<RawValue>>,
    request_params: &BTreeMap<String, Box<RawValue>>,
    region_inputs: RegionInputs<'_>,
) -> Result<String, BatchCreateError> {
    let params = merged_params(deployment_params, request_params)?;
    let region = resolve_region(&params, Some(model), region_inputs)?;
    let suffix = get_aws_partition(Some(&region)).dns_suffix;
    Ok(format!(
        "https://bedrock.{region}.{suffix}/model-invocation-job"
    ))
}

pub async fn create_signed_request<F, Fut>(
    input: SignedCreateInput<'_>,
    resolve_credentials: F,
) -> Result<SignedBatchRequest, BatchCreateError>
where
    F: FnOnce(&AwsSettings, &str) -> Fut,
    Fut: Future<Output = Result<Credentials, CredentialError>>,
{
    let url = complete_batch_url(
        input.model,
        input.deployment_params,
        input.request_params,
        input.region_inputs,
    )?;
    let params = merged_params(input.deployment_params, input.request_params)?;
    let text = serde_json::to_string(input.request).map_err(BatchCreateError::RequestJson)?;
    let data: IndexMap<String, Json> =
        serde_json::from_str(&text).map_err(BatchCreateError::RequestJson)?;
    let signed: AwsSignedRequest = sign_batch_request(
        BatchRequest {
            service_name: "bedrock",
            data: BatchData::Json(&data),
            endpoint: &url,
            optional_params: &params,
            method: "POST",
            region_inputs: input.region_inputs,
            now: input.now,
        },
        resolve_credentials,
    )
    .await?;
    let body = String::from_utf8(signed.body).map_err(BatchCreateError::SignedBody)?;
    Ok(SignedBatchRequest {
        method: signed.method,
        url: signed.url,
        headers: signed.headers.into_iter().collect(),
        data: body,
    })
}

pub async fn transform_create_batch_request<F, Fut>(
    input: CreateBatchInput<'_>,
    resolve_credentials: F,
) -> Result<SignedBatchRequest, BatchCreateError>
where
    F: FnOnce(&AwsSettings, &str) -> Fut,
    Fut: Future<Output = Result<Credentials, CredentialError>>,
{
    let request = build_create_request(&input)?;
    let signed = SignedCreateInput {
        model: input.model,
        request: &request,
        deployment_params: input.deployment_params,
        request_params: input.request_params,
        region_inputs: input.region_inputs,
        now: input.now,
    };
    create_signed_request(signed, resolve_credentials).await
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

fn merged_params(
    deployment: &BTreeMap<String, Box<RawValue>>,
    request: &BTreeMap<String, Box<RawValue>>,
) -> Result<BTreeMap<String, Box<RawValue>>, BatchCreateError> {
    let merged = merge_aws_request_params(&policy_params(deployment), &policy_params(request));
    let mut signing = BTreeMap::new();
    for (name, value) in merged {
        let text = match value {
            RequestValue::Raw(text) => text,
            other
            @ (RequestValue::Text(_) | RequestValue::Secret(_) | RequestValue::Mapping(_)) => {
                serde_json::to_string(&policy_json(other)).map_err(BatchCreateError::RequestJson)?
            }
        };
        let raw = RawValue::from_string(text).map_err(BatchCreateError::RequestJson)?;
        signing.insert(name, raw);
    }
    Ok(signing)
}

fn policy_params(params: &BTreeMap<String, Box<RawValue>>) -> Params {
    let mut built = Params::new();
    for (name, raw) in params {
        let value = serde_json::from_str::<Json>(raw.get()).unwrap_or(Json::Null);
        let documented = match value {
            Json::Text(text) => RequestValue::Text(text),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
                RequestValue::Raw(raw.get().to_owned())
            }
        };
        built.insert(name.clone(), documented);
    }
    built
}

fn policy_json(value: RequestValue) -> Json {
    match value {
        RequestValue::Text(text) => Json::Text(text),
        RequestValue::Secret(secret) => Json::Text(secret.expose_secret().to_owned()),
        RequestValue::Mapping(fields) => Json::Object(
            fields
                .into_iter()
                .map(|(name, entry)| (name, policy_json(entry)))
                .collect(),
        ),
        RequestValue::Raw(text) => serde_json::from_str(&text).unwrap_or(Json::Null),
    }
}
