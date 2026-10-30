use crate::s3_transport::option_value::OptionValue;
use crate::s3_v2::flag::PythonBool;
use secrecy::SecretString;
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Source<Config: Default> {
    #[serde(rename = "s3_bucket_name")]
    pub bucket_name: Option<String>,
    #[serde(rename = "s3_region_name")]
    pub region_name: Option<String>,
    #[serde(rename = "s3_api_version")]
    pub api_version: Option<String>,
    #[serde(rename = "s3_endpoint_url")]
    pub endpoint_url: Option<String>,
    #[serde(rename = "s3_aws_access_key_id")]
    pub aws_access_key_id: Option<SecretString>,
    #[serde(rename = "s3_aws_secret_access_key")]
    pub aws_secret_access_key: Option<SecretString>,
    #[serde(rename = "s3_aws_session_token")]
    pub aws_session_token: Option<SecretString>,
    #[serde(rename = "s3_aws_session_name")]
    pub aws_session_name: Option<String>,
    #[serde(rename = "s3_aws_profile_name")]
    pub aws_profile_name: Option<String>,
    #[serde(rename = "s3_aws_role_name")]
    pub aws_role_name: Option<String>,
    #[serde(rename = "s3_aws_web_identity_token")]
    pub aws_web_identity_token: Option<SecretString>,
    #[serde(rename = "s3_aws_sts_endpoint")]
    pub aws_sts_endpoint: Option<String>,
    #[serde(rename = "s3_path")]
    pub path: Option<String>,
    #[serde(rename = "s3_server_side_encryption")]
    pub server_side_encryption: Option<OptionValue>,
    #[serde(rename = "s3_sse_kms_key_id")]
    pub sse_kms_key_id: Option<OptionValue>,
    #[serde(rename = "s3_use_ssl")]
    pub use_ssl: Option<OptionValue>,
    #[serde(rename = "s3_verify")]
    pub verify: Option<OptionValue>,
    #[serde(rename = "s3_use_team_prefix")]
    pub use_team_prefix: Option<PythonBool>,
    #[serde(rename = "s3_strip_base64_files")]
    pub strip_base64_files: Option<PythonBool>,
    #[serde(rename = "s3_use_key_prefix")]
    pub use_key_prefix: Option<PythonBool>,
    #[serde(rename = "s3_use_virtual_hosted_style")]
    pub use_virtual_hosted_style: Option<PythonBool>,
    #[serde(rename = "s3_log_prompts_only")]
    pub log_prompts_only: Option<OptionValue>,
    #[serde(rename = "s3_partition_granularity")]
    pub partition_granularity: Option<OptionValue>,
    #[serde(rename = "s3_max_concurrent_uploads")]
    pub max_concurrent_uploads: Option<OptionValue>,
    #[serde(rename = "s3_max_queue_size")]
    pub max_queue_size: Option<OptionValue>,
    #[serde(rename = "s3_max_retry_age_seconds")]
    pub max_retry_age_seconds: Provided<OptionValue>,
    #[serde(rename = "s3_drop_on_terminal_error")]
    pub drop_on_terminal_error: Option<OptionValue>,
    #[serde(rename = "s3_adaptive_concurrency")]
    pub adaptive_concurrency: Option<OptionValue>,
    #[serde(rename = "s3_max_adaptive_concurrency")]
    pub max_adaptive_concurrency: Option<OptionValue>,
    #[serde(rename = "s3_batch_file_upload")]
    pub batch_file_upload: Option<OptionValue>,
    #[serde(rename = "s3_config")]
    pub config: Option<BTreeMap<String, Config>>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(untagged)]
pub enum Provided<Value> {
    Present(Option<Value>),
    #[default]
    Missing,
}
