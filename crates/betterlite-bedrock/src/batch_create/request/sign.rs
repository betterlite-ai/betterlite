use std::collections::BTreeMap;
use std::future::Future;
use std::time::SystemTime;

use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_context::aws_partition::get_aws_partition;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::ExposeSecret as _;
use serde_json::value::RawValue;

use super::build::{CreateBatchInput, build_create_request};
use super::wire::{BedrockCreateRequest, SignedBatchRequest};
use crate::aws_settings::sources::Credentials;
use crate::batch_create::BatchCreateError;
use crate::credentials::error::CredentialError;
use crate::request_policy::aws_params::merge_aws_request_params;
use crate::request_policy::{Params, RequestValue};
use crate::signing::batch::{BatchData, BatchRequest, sign_batch_request};
use crate::signing::request::AwsSignedRequest;
use crate::signing::target::{RegionInputs, resolve_region};

pub struct SignedCreateInput<'text> {
    pub model: &'text str,
    pub request: &'text BedrockCreateRequest,
    pub deployment_params: &'text BTreeMap<String, Box<RawValue>>,
    pub request_params: &'text BTreeMap<String, Box<RawValue>>,
    pub region_inputs: RegionInputs<'text>,
    pub now: SystemTime,
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
