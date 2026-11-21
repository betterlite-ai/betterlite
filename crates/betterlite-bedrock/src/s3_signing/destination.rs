use std::collections::BTreeMap;

use betterlite_config::sections::bedrock_services_s3_access::BUCKET_NAME_ENV;
use betterlite_config::sections::bedrock_services_s3_signing::{
    ENDPOINT_URL_PARAM, REGION_NAME_PARAM,
};
use betterlite_context::aws_partition::get_aws_partition;
use betterlite_context::cloud_storage::{Encoding, encode_object_name, split_bucket_name};
use betterlite_openai::files::records::CreatePurpose;
use betterlite_prompts::media_metadata::{FileData, extract_file_data};
use uuid::Uuid;

use super::naming::get_object_name;
use super::{S3SigningError, param_text};
use crate::aws_endpoints::region::{LlmRegion, llm_region_name};
use crate::request_policy::Params;
use crate::request_policy::aws_params::merge_aws_request_params;
use crate::signing::target::RegionInputs;

const BUCKET_NAME_PARAM: &str = "s3_bucket_name";
const AWS_REGION_PARAM: &str = "aws_region_name";

// no Debug: litellm_params may hold AWS keys as plain text values
#[derive(Clone, Copy)]
pub struct UploadTarget<'source> {
    pub model: &'source str,
    pub litellm_params: &'source Params,
    pub optional_params: &'source Params,
    pub file: Option<&'source FileData>,
    pub purpose: Option<CreatePurpose>,
    pub environment: &'source BTreeMap<String, String>,
    pub region_inputs: RegionInputs<'source>,
    pub id: Uuid,
}

// the upload bucket comes from litellm_params as sent; only downloads require B643's trusted snapshot
fn upload_bucket(target: &UploadTarget<'_>) -> Result<String, S3SigningError> {
    let configured = param_text(target.litellm_params, BUCKET_NAME_PARAM)?
        .filter(|bucket| !bucket.is_empty())
        .or_else(|| {
            target
                .environment
                .get(BUCKET_NAME_ENV)
                .filter(|bucket| !bucket.is_empty())
                .cloned()
        });
    configured.ok_or(S3SigningError::BucketRequired)
}

// s3_region_name is used as given; only the aws_region_name fallback is validated
fn upload_region(
    target: &UploadTarget<'_>,
    request_params: &Params,
) -> Result<String, S3SigningError> {
    for params in [target.litellm_params, target.optional_params] {
        if let Some(region) = param_text(params, REGION_NAME_PARAM)?.filter(|name| !name.is_empty())
        {
            return Ok(region);
        }
    }
    let explicit = param_text(request_params, AWS_REGION_PARAM)?;
    llm_region_name(LlmRegion {
        explicit: explicit.as_deref(),
        model: Some(target.model),
        model_id: None,
        environment_region_name: target.region_inputs.environment_region_name,
        environment_region: target.region_inputs.environment_region,
        session_region: target.region_inputs.session_region,
    })
    .map_err(S3SigningError::Region)
}

pub fn get_complete_file_url(target: &UploadTarget<'_>) -> Result<String, S3SigningError> {
    let request_params = merge_aws_request_params(target.litellm_params, target.optional_params);
    let (bucket, prefix) = split_bucket_name(&upload_bucket(target)?)?;
    let region = upload_region(target, &request_params)?;
    let file = target.file.ok_or(S3SigningError::FileRequired)?;
    let purpose = target.purpose.ok_or(S3SigningError::PurposeRequired)?;
    let extracted = extract_file_data(file)?;
    let object_name = get_object_name(&extracted, purpose, target.id)?;
    let prefixed = if prefix.is_empty() {
        object_name
    } else {
        format!("{prefix}/{object_name}")
    };
    let encoded = encode_object_name(&prefixed, Encoding::S3);
    let endpoint = param_text(&request_params, ENDPOINT_URL_PARAM)?
        .filter(|url| !url.is_empty())
        .unwrap_or_else(|| {
            format!(
                "https://s3.{region}.{}",
                get_aws_partition(Some(&region)).dns_suffix
            )
        });
    Ok(format!(
        "{}/{bucket}/{encoded}",
        endpoint.trim_end_matches('/')
    ))
}
