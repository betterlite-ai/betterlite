use crate::aws_endpoints::region::validate_region;
use crate::batch_status::handler::region_from_bedrock_arn;

use super::error::BatchCallError;

pub const DEFAULT_REGION: &str = "us-east-1";

// LiteLLM reads an explicit region first, then the ARN, and only then its boto3 default
pub fn resolve_region(explicit: Option<&str>, batch_id: &str) -> Result<String, BatchCallError> {
    let region = match explicit {
        Some(region) => region.to_owned(),
        None => region_from_bedrock_arn(batch_id).unwrap_or_else(|| DEFAULT_REGION.to_owned()),
    };
    validate_region(Some(&region))?;
    Ok(region)
}
