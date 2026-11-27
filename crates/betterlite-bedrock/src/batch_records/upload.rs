use std::collections::BTreeMap;
use std::fmt::{self, Debug, Formatter};
use std::future::Future;
use std::str;
use std::time::SystemTime;

use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_logging::python_json;
use betterlite_openai::files::records::CreatePurpose;
use betterlite_prompts::media_metadata::FileData;
use betterlite_prompts::media_metadata::extract_file_data;
use betterlite_secret_managers::secret_lookup::SecretLookup;
use uuid::Uuid;

use super::error::BatchRecordsError;
use super::jsonl::{BatchRecordContext, transform_openai_jsonl};
use super::record::{LINE_BOUNDARIES, decode_batch_input_line};
use crate::aws_settings::sources::Credentials;
use crate::bedrock_model_capabilities::ModelInfoLookup;
use crate::credentials::error::CredentialError;
use crate::request_policy::aws_params::{merge_aws_request_params, raw_params};
use crate::request_policy::{Params, RequestValue};
use crate::s3_access::settings::{
    S3Setting, S3SettingSources, SettingMap, SettingValue, resolve_s3_setting,
};
use crate::s3_signing::destination::{UploadTarget, get_complete_file_url};
use crate::s3_signing::signer::{S3PutRequest, sign_s3_request};
use crate::s3_signing::{UPLOAD_CONTENT_LENGTH_PARAM, UPLOAD_URL_PARAM};
use crate::signing::request::AwsSignedRequest;
use crate::signing::target::RegionInputs;

const AWS_REGION_NAME: &str = "aws_region_name";
const LITELLM_MODEL_PARAM: &str = "model";
const S3_REGION_NAME: &str = "s3_region_name";

pub struct CreateFileRequest<'source> {
    pub model: &'source str,
    pub file: Option<&'source FileData>,
    pub purpose: Option<CreatePurpose>,
    pub optional_params: &'source Params,
    pub litellm_params: &'source mut Params,
    pub environment: &'source BTreeMap<String, String>,
    pub secrets: &'source SecretLookup,
    pub region_inputs: RegionInputs<'source>,
    pub id: Uuid,
    pub now: SystemTime,
}

pub struct FileUploadHandoff {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub data: String,
}

// transform_create_file_request: convert a batch JSONL upload, sign it for S3 and hand the PUT to the caller.
pub async fn transform_create_file_request<Lookup, Resolver, Fut>(
    request: &mut CreateFileRequest<'_>,
    context: &BatchRecordContext<'_, Lookup>,
    resolve_credentials: Resolver,
) -> Result<FileUploadHandoff, BatchRecordsError>
where
    Lookup: ModelInfoLookup,
    Resolver: FnOnce(&AwsSettings, &str) -> Fut,
    Fut: Future<Output = Result<Credentials, CredentialError>>,
{
    let file_content = upload_content(request, context).await?;
    let api_base = complete_file_url(request)?;
    let signed = sign_content(request, &file_content, &api_base, resolve_credentials).await?;
    request
        .litellm_params
        .insert(String::from(UPLOAD_URL_PARAM), RequestValue::Text(api_base));
    request.litellm_params.insert(
        String::from(UPLOAD_CONTENT_LENGTH_PARAM),
        RequestValue::Raw(file_content.len().to_string()),
    );
    let data = String::from_utf8(signed.body)
        .map_err(|source| BatchRecordsError::Utf8(source.utf8_error()))?;
    Ok(FileUploadHandoff {
        method: signed.method,
        url: signed.url,
        headers: signed.headers,
        data,
    })
}

async fn upload_content<Lookup>(
    request: &CreateFileRequest<'_>,
    context: &BatchRecordContext<'_, Lookup>,
) -> Result<String, BatchRecordsError>
where
    Lookup: ModelInfoLookup,
{
    let file = request.file.ok_or(BatchRecordsError::FileRequired)?;
    let extracted = extract_file_data(file)?;
    let is_batch = context.families.is_batch_jsonl(
        request.purpose.map(CreatePurpose::wire_name),
        Some(extracted.content_type.as_str()),
        Some(extracted.content.as_slice()),
    );
    if is_batch {
        batch_content(
            &extracted.content,
            target_model(request.model, &*request.litellm_params),
            context,
        )
        .await
    } else {
        Ok(str::from_utf8(&extracted.content)?.to_owned())
    }
}

fn complete_file_url(request: &CreateFileRequest<'_>) -> Result<String, BatchRecordsError> {
    get_complete_file_url(&UploadTarget {
        model: request.model,
        litellm_params: &*request.litellm_params,
        optional_params: request.optional_params,
        file: request.file,
        purpose: request.purpose,
        environment: request.environment,
        region_inputs: request.region_inputs,
        id: request.id,
    })
    .map_err(BatchRecordsError::from)
}

async fn sign_content<Resolver, Fut>(
    request: &CreateFileRequest<'_>,
    content: &str,
    api_base: &str,
    resolve_credentials: Resolver,
) -> Result<AwsSignedRequest, BatchRecordsError>
where
    Resolver: FnOnce(&AwsSettings, &str) -> Fut,
    Fut: Future<Output = Result<Credentials, CredentialError>>,
{
    let request_params = signing_params(&*request.litellm_params, request.optional_params);
    let deployment_settings = setting_map(&*request.litellm_params);
    let call_settings = setting_map(&request_params);
    let encryption_key_id = resolve_s3_setting(
        S3Setting::EncryptionKeyId,
        &S3SettingSources {
            params: &deployment_settings,
            optional_params: &call_settings,
            secrets: request.secrets,
            environment: request.environment,
        },
    )?;
    let encoded_params = raw_params(&request_params).map_err(BatchRecordsError::Wire)?;
    sign_s3_request(
        S3PutRequest {
            content,
            api_base,
            optional_params: &encoded_params,
            encryption_key_id: encryption_key_id.as_ref(),
            region_inputs: request.region_inputs,
            now: request.now,
        },
        resolve_credentials,
    )
    .await
    .map_err(BatchRecordsError::from)
}

async fn batch_content<Lookup>(
    payload: &[u8],
    target_model: &str,
    context: &BatchRecordContext<'_, Lookup>,
) -> Result<String, BatchRecordsError>
where
    Lookup: ModelInfoLookup,
{
    let text = str::from_utf8(payload)?;
    let mut records = Vec::new();
    for line in text.split(LINE_BOUNDARIES) {
        if let Some(record) = decode_batch_input_line(line.as_bytes())? {
            records.push(record);
        }
    }
    let transformed = transform_openai_jsonl(&records, target_model, context).await?;
    let lines: Vec<String> = transformed
        .iter()
        .map(|record| python_json::dumps(&record.to_json()))
        .collect();
    Ok(lines.join("\n"))
}

// `model or (litellm_params.get("model") if str else "")`.
fn target_model<'source>(model: &'source str, litellm_params: &'source Params) -> &'source str {
    if !model.is_empty() {
        return model;
    }
    match litellm_params.get(LITELLM_MODEL_PARAM) {
        Some(RequestValue::Text(text)) => text,
        Some(RequestValue::Secret(_) | RequestValue::Mapping(_) | RequestValue::Raw(_)) | None => {
            ""
        }
    }
}

// merge_bedrock_aws_request_params, then s3_region_name overwrites aws_region_name unconditionally.
fn signing_params(litellm_params: &Params, optional_params: &Params) -> Params {
    let mut merged = merge_aws_request_params(litellm_params, optional_params);
    if let Some(s3_region) = text_param(litellm_params, S3_REGION_NAME)
        .or_else(|| text_param(optional_params, S3_REGION_NAME))
    {
        merged.insert(
            String::from(AWS_REGION_NAME),
            RequestValue::Text(s3_region.to_owned()),
        );
    }
    merged
}

fn text_param<'source>(params: &'source Params, name: &str) -> Option<&'source str> {
    match params.get(name) {
        Some(RequestValue::Text(text)) if !text.is_empty() => Some(text),
        Some(
            RequestValue::Text(_)
            | RequestValue::Secret(_)
            | RequestValue::Mapping(_)
            | RequestValue::Raw(_),
        )
        | None => None,
    }
}

fn setting_map(params: &Params) -> SettingMap {
    params
        .iter()
        .map(|(name, value)| {
            let setting = match value {
                RequestValue::Text(text) => SettingValue::Text(text.clone()),
                RequestValue::Secret(_) | RequestValue::Mapping(_) | RequestValue::Raw(_) => {
                    SettingValue::Other
                }
            };
            (name.clone(), setting)
        })
        .collect()
}

// The handoff carries signed headers and the converted content, so Debug lists header names and lengths only.
impl Debug for FileUploadHandoff {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let names: Vec<&str> = self
            .headers
            .iter()
            .map(|(name, _value)| name.as_str())
            .collect();
        f.debug_struct("FileUploadHandoff")
            .field("method", &self.method)
            .field("url", &self.url)
            .field("header_names", &names)
            .field("data_length", &self.data.len())
            .finish()
    }
}
