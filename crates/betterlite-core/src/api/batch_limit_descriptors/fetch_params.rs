use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use super::BatchLimitDescriptors;
use super::routing::{BatchCredentialError, credentials_for, provider_from, text_member};
use crate::api::file_routing_ids::routing::{decode_model_from_file_id, get_original_file_id};
use crate::api::model_credentials::CredentialSource;
use crate::api::request_destination::RequestData;

// _extract_file_access_credentials' list: the auth and client settings a provider file read needs
const CREDENTIAL_KEYS: &str = concat!(
    "api_key api_base api_version organization azure_ad_token azure_ad_token_provider ",
    "vertex_project vertex_location vertex_credentials gcs_bucket_name bucket_name ",
    "s3_endpoint_url s3_region_name timeout max_retries _litellm_internal_model_credentials ",
    "aws_access_key_id aws_bedrock_project_id aws_bedrock_runtime_endpoint aws_external_id ",
    "aws_profile_name aws_region_name aws_role_name aws_secret_access_key aws_session_name ",
    "aws_session_tags aws_session_token aws_sts_endpoint aws_web_identity_token",
);

fn apply_credentials(params: &mut JsonObject, credentials: &JsonObject, model: &str) {
    for name in CREDENTIAL_KEYS.split_whitespace() {
        if let Some(value) = credentials.get(name) {
            params.insert(name.to_owned(), value.clone());
        }
    }
    params.insert("model".to_owned(), Json::Text(model.to_owned()));
    if let Some(provider) = provider_from(credentials) {
        params.insert("custom_llm_provider".to_owned(), Json::Text(provider));
    }
}

// A model-embedded file id fetches by its original id; every other id stays as it came in
pub(super) fn fetch_params<Limiter, Credentials>(
    service: &BatchLimitDescriptors<Limiter, Credentials>,
    file_id: &str,
    custom_llm_provider: &str,
    data: &RequestData,
) -> Result<(String, JsonObject), BatchCredentialError<Credentials::Error>>
where
    Credentials: CredentialSource,
{
    let mut params = JsonObject::new();
    params.insert(
        "custom_llm_provider".to_owned(),
        Json::Text(custom_llm_provider.to_owned()),
    );
    if let Some(model) = decode_model_from_file_id(file_id) {
        if let Some(credentials) = credentials_for(service.credentials.as_ref(), &model)? {
            apply_credentials(&mut params, &credentials, &model);
        }
        return Ok((get_original_file_id(file_id), params));
    }
    if let Some(model) = text_member(data, "model")
        && let Some(credentials) = credentials_for(service.credentials.as_ref(), model)?
    {
        apply_credentials(&mut params, &credentials, model);
    }
    Ok((file_id.to_owned(), params))
}
