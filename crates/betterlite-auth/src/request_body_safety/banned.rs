use betterlite_models::model_registration::JsonObject;

use crate::request_ban_catalog::RequestBanCatalog;

use super::grants;
use super::{RequestBodySafety, SafetyError};

// URL-valued fields stay refused under either opt-in, the architect's security decision for B884.
pub const URL_DESTINATION_FIELDS: [&str; 16] = [
    "api_base",
    "base_url",
    "aws_sts_endpoint",
    "aws_bedrock_runtime_endpoint",
    "s3_endpoint_url",
    "sagemaker_base_url",
    "deployment_url",
    "langfuse_host",
    "langsmith_base_url",
    "posthog_host",
    "posthog_api_url",
    "braintrust_host",
    "slack_webhook_url",
    "signoz_ingestion_endpoint",
    "dd_site",
    "dd_agent_host",
];

// _check_banned_params: the first present field no opt-in admits fails, a null value included.
pub fn check(
    scan: &RequestBodySafety<'_>,
    body: &JsonObject,
    catalog: &RequestBanCatalog,
) -> Result<(), SafetyError> {
    for param in &catalog.banned {
        let Some(value) = body.get(*param) else {
            continue;
        };
        if URL_DESTINATION_FIELDS.contains(param) {
            return Err(SafetyError::Banned {
                param: (*param).to_owned(),
            });
        }
        if scan.allow_client_side_credentials {
            return Ok(());
        }
        if grants::model_level_allows(scan.model, param, value, scan.grants)? {
            continue;
        }
        return Err(SafetyError::Banned {
            param: (*param).to_owned(),
        });
    }
    Ok(())
}
