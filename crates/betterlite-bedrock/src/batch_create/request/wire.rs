use std::collections::BTreeMap;
use std::fmt::{self, Debug, Formatter};

use serde::Serialize;

use crate::batch_create::tags::BedrockTag;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct S3InputDataConfig {
    #[serde(rename = "s3Uri")]
    pub s3_uri: String,
    #[serde(rename = "s3BucketOwner", skip_serializing_if = "Option::is_none")]
    pub s3_bucket_owner: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct S3OutputDataConfig {
    #[serde(rename = "s3Uri")]
    pub s3_uri: String,
    #[serde(rename = "s3BucketOwner", skip_serializing_if = "Option::is_none")]
    pub s3_bucket_owner: Option<String>,
    #[serde(rename = "s3EncryptionKeyId", skip_serializing_if = "Option::is_none")]
    pub s3_encryption_key_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InputDataConfig {
    #[serde(rename = "s3InputDataConfig")]
    pub s3_input_data_config: S3InputDataConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OutputDataConfig {
    #[serde(rename = "s3OutputDataConfig")]
    pub s3_output_data_config: S3OutputDataConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BedrockCreateRequest {
    #[serde(rename = "modelId")]
    pub model_id: String,
    #[serde(rename = "jobName")]
    pub job_name: String,
    #[serde(rename = "inputDataConfig")]
    pub input_data_config: InputDataConfig,
    #[serde(rename = "outputDataConfig")]
    pub output_data_config: OutputDataConfig,
    #[serde(rename = "roleArn")]
    pub role_arn: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<BedrockTag>>,
    #[serde(
        rename = "timeoutDurationInHours",
        skip_serializing_if = "Option::is_none"
    )]
    pub timeout_duration_in_hours: Option<i64>,
}

#[derive(Clone, PartialEq, Eq)]
pub struct SignedBatchRequest {
    pub method: String,
    pub url: String,
    pub headers: BTreeMap<String, String>,
    pub data: String,
}

// header values and the signed body never reach Debug, since a session token rides in the headers
impl Debug for SignedBatchRequest {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let names: Vec<&str> = self.headers.keys().map(String::as_str).collect();
        f.debug_struct("SignedBatchRequest")
            .field("method", &self.method)
            .field("url", &self.url)
            .field("header_names", &names)
            .field("data_length", &self.data.len())
            .finish()
    }
}
