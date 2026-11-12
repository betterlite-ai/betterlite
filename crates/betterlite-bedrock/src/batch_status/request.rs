use std::collections::BTreeMap;
use std::future::Future;
use std::time::SystemTime;

use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_context::aws_partition::{get_aws_partition, is_bedrock_arn};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use percent_encoding::utf8_percent_encode;
use secrecy::ExposeSecret as _;
use serde_json::value::RawValue;

use crate::aws_endpoints::model_id::MODEL_ID_SAFE;
use crate::aws_settings::sources::Credentials;
use crate::batch_status::error::BatchStatusError;
use crate::credentials::error::CredentialError;
use crate::request_policy::aws_params::merge_aws_request_params;
use crate::request_policy::{Params, RequestValue};
use crate::signing::batch::{BatchData, BatchRequest, sign_batch_request};
use crate::signing::request::AwsSignedRequest;
use crate::signing::target::RegionInputs;

#[derive(Debug, Clone, Copy)]
pub struct RetrieveRequest<'source> {
    pub batch_id: &'source str,
    pub litellm_params: &'source Params,
    pub optional_params: &'source Params,
    pub now: SystemTime,
}

pub async fn retrieve_signed_request<F, Fut>(
    request: RetrieveRequest<'_>,
    resolve_credentials: F,
) -> Result<AwsSignedRequest, BatchStatusError>
where
    F: FnOnce(&AwsSettings, &str) -> Fut,
    Fut: Future<Output = Result<Credentials, CredentialError>>,
{
    let region = arn_region(request.batch_id)?;
    let suffix = get_aws_partition(Some(&region)).dns_suffix;
    let encoded = utf8_percent_encode(request.batch_id, &MODEL_ID_SAFE);
    let url = format!("https://bedrock.{region}.{suffix}/model-invocation-job/{encoded}");
    let merged = merge_aws_request_params(request.litellm_params, request.optional_params);
    let optional_params = signing_params(&merged)?;
    let data = IndexMap::new();
    sign_batch_request(
        BatchRequest {
            service_name: "bedrock",
            data: BatchData::Json(&data),
            endpoint: &url,
            optional_params: &optional_params,
            method: "GET",
            region_inputs: RegionInputs::default(),
            now: request.now,
        },
        resolve_credentials,
    )
    .await
    .map_err(|source| BatchStatusError::Signing { source })
}

fn arn_region(batch_id: &str) -> Result<String, BatchStatusError> {
    if !is_bedrock_arn(batch_id) {
        return Err(BatchStatusError::BatchId {
            batch_id: batch_id.to_owned(),
        });
    }
    let parts: Vec<&str> = batch_id.split(':').collect();
    if parts.len() < 6 {
        return Err(BatchStatusError::ArnFormat {
            batch_id: batch_id.to_owned(),
        });
    }
    let region = parts
        .get(3)
        .copied()
        .ok_or_else(|| BatchStatusError::ArnFormat {
            batch_id: batch_id.to_owned(),
        })?;
    if !is_region(region) {
        return Err(BatchStatusError::Region {
            batch_id: batch_id.to_owned(),
        });
    }
    Ok(region.to_owned())
}

// LiteLLM's ^[a-z][a-z0-9-]*$ keeps an ARN-supplied region from choosing another host
fn is_region(value: &str) -> bool {
    let mut characters = value.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    first.is_ascii_lowercase()
        && characters.all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
        })
}

fn signing_params(params: &Params) -> Result<BTreeMap<String, Box<RawValue>>, BatchStatusError> {
    params
        .iter()
        .map(|(name, value)| {
            let source = serde_json::to_string(&value_json(value)?)
                .map_err(|error| BatchStatusError::Parameters { source: error })?;
            let encoded = RawValue::from_string(source)
                .map_err(|error| BatchStatusError::Parameters { source: error })?;
            Ok((name.clone(), encoded))
        })
        .collect()
}

fn value_json(value: &RequestValue) -> Result<Json, BatchStatusError> {
    match value {
        RequestValue::Text(text) => Ok(Json::Text(text.clone())),
        RequestValue::Secret(secret) => Ok(Json::Text(secret.expose_secret().to_owned())),
        RequestValue::Raw(raw) => {
            serde_json::from_str(raw).map_err(|source| BatchStatusError::Parameters { source })
        }
        RequestValue::Mapping(entries) => {
            let mut object = IndexMap::with_capacity(entries.len());
            for (name, entry) in entries {
                object.insert(name.clone(), value_json(entry)?);
            }
            Ok(Json::Object(object))
        }
    }
}
