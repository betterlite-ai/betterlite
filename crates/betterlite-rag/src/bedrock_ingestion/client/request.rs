use std::error::Error as StdError;

use betterlite_config::sections::bedrock_settings::AwsSettings;
use serde::de::DeserializeOwned;

use crate::bedrock_ingestion::error::BedrockIngestionError;
use crate::bedrock_ingestion::settings::BedrockRagSettings;

const JSON_CONTENT_TYPE: &str = "application/json";

pub(super) fn json_headers(has_body: bool) -> Vec<(String, String)> {
    if !has_body {
        return Vec::new();
    }
    vec![("Content-Type".to_owned(), JSON_CONTENT_TYPE.to_owned())]
}

// the ten BaseAWSLLM.get_credentials keys, plus the region every call resolves them with
pub(super) fn aws_settings(settings: &BedrockRagSettings) -> AwsSettings {
    AwsSettings {
        access_key_id: settings.credentials.access_key_id.clone(),
        secret_access_key: settings.credentials.secret_access_key.clone(),
        session_token: settings.credentials.session_token.clone(),
        region_name: Some(settings.aws_region_name.clone()),
        session_name: settings.credentials.session_name.clone(),
        profile_name: settings.credentials.profile_name.clone(),
        role_name: settings.credentials.role_name.clone(),
        web_identity_token: settings.credentials.web_identity_token.clone(),
        sts_endpoint: settings.credentials.sts_endpoint.clone(),
        external_id: settings.credentials.external_id.clone(),
        session_tags: None,
        role_arn: None,
        web_identity_token_file: None,
        bearer_token: None,
    }
}

pub(super) fn parse_response<Response, TransportFailure>(
    operation: &'static str,
    body: &[u8],
) -> Result<Response, BedrockIngestionError<TransportFailure>>
where
    Response: DeserializeOwned,
    TransportFailure: StdError + 'static,
{
    serde_json::from_slice(body)
        .map_err(|source| BedrockIngestionError::Response { operation, source })
}
