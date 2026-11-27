use std::time::SystemTime;

use betterlite_context::aws_partition::get_aws_partition;
use percent_encoding::utf8_percent_encode;

use crate::aws_endpoints::model_id::MODEL_ID_SAFE;
use crate::aws_settings::sources::Credentials;
use crate::batch_status::handler::extract_job_id_from_arn;
use crate::signing::error::SigningError;
use crate::signing::headers::{merge_computed, prepare, without_computed};
use crate::signing::request::AwsSignedRequest;
use crate::signing::sigv4::{Signable, sign_request};

use super::transport::BEDROCK_SERVICE;

// the control plane serves both operations from /model-invocation-job/{jobIdentifier}
pub fn job_url(region: &str, batch_id: &str) -> String {
    let suffix = get_aws_partition(Some(region)).dns_suffix;
    let encoded = utf8_percent_encode(batch_id, &MODEL_ID_SAFE);
    format!("https://bedrock.{region}.{suffix}/model-invocation-job/{encoded}")
}

// the logged path keeps a bare job id and otherwise falls back to the whole identifier
#[must_use]
pub fn logging_url(region: &str, batch_id: &str) -> String {
    let suffix = get_aws_partition(Some(region)).dns_suffix;
    let path_id = extract_job_id_from_arn(batch_id).unwrap_or_else(|| batch_id.to_owned());
    format!("https://bedrock.{region}.{suffix}/model-invocation-job/{path_id}")
}

pub fn sign_stop(
    credentials: &Credentials,
    region: &str,
    batch_id: &str,
    now: SystemTime,
) -> Result<AwsSignedRequest, SigningError> {
    let url = format!("{}/stop", job_url(region, batch_id));
    // botocore sends no body and no content type when the only input member is the URI label
    let supplied = without_computed(Vec::new(), credentials.session_token.is_some());
    let computed = sign_request(&Signable {
        method: "POST",
        url: &url,
        headers: &supplied,
        body: &[],
        service: BEDROCK_SERVICE,
        region,
        credentials,
        now,
    })?;
    let merged = merge_computed(supplied, computed);
    Ok(AwsSignedRequest {
        method: "POST".to_owned(),
        url,
        headers: prepare(merged, 0, "POST"),
        body: Vec::new(),
    })
}
