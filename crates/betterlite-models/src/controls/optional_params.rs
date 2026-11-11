use secrecy::SecretString;
use serde::Deserialize;

use crate::controls::json_value::JsonValue;
use crate::price_fields::PriceFields;

// The deployment keys LiteLLM copies into litellm_params only when kwargs carry them; betterlite takes them from trusted config and server code, never from a request.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct OptionalParams {
    pub azure_ad_token: Option<SecretString>,
    pub tenant_id: Option<String>,
    pub client_id: Option<String>,
    pub client_secret: Option<SecretString>,
    pub azure_username: Option<String>,
    pub azure_password: Option<SecretString>,
    pub azure_scope: Option<String>,
    pub vertex_project: Option<String>,
    pub vertex_location: Option<String>,
    pub vertex_credentials: Option<JsonValue>,
    pub vertex_ai_project: Option<String>,
    pub vertex_ai_location: Option<String>,
    pub vertex_ai_credentials: Option<JsonValue>,
    pub gcs_bucket_name: Option<String>,
    pub bucket_name: Option<String>,
    pub s3_endpoint_url: Option<String>,
    pub s3_region_name: Option<String>,
    pub s3_access_key_id: Option<SecretString>,
    pub s3_secret_access_key: Option<SecretString>,
    pub gigachat_scope: Option<String>,
    pub gigachat_auth_url: Option<String>,
    pub gigachat_access_token: Option<SecretString>,
    pub timeout: Option<f64>,
    pub client_side_timeout: Option<f64>,
    pub tpm: Option<u32>,
    pub rpm: Option<u32>,
    pub itpm: Option<u32>,
    pub otpm: Option<u32>,
    pub use_xai_oauth: Option<bool>,
    pub provider_affinity_header: Option<String>,
    pub aws_region_name: Option<String>,
    pub aws_access_key_id: Option<SecretString>,
    pub aws_secret_access_key: Option<SecretString>,
    pub aws_session_token: Option<SecretString>,
    pub aws_session_name: Option<String>,
    pub aws_profile_name: Option<String>,
    pub aws_role_name: Option<String>,
    pub aws_web_identity_token: Option<SecretString>,
    pub aws_sts_endpoint: Option<String>,
    pub aws_external_id: Option<String>,
    pub aws_session_tags: Option<Vec<AwsSessionTag>>,
    pub aws_bedrock_runtime_endpoint: Option<String>,
    pub aws_bedrock_project_id: Option<String>,
    #[serde(flatten)]
    pub pricing: PriceFields,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct AwsSessionTag {
    #[serde(rename = "Key")]
    pub key: String,
    #[serde(rename = "Value")]
    pub value: String,
}
