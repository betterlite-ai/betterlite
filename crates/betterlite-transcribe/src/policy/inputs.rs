use std::collections::BTreeMap;

use crate::field::Field;
use serde::Deserialize;
use serde::de::IgnoredAny;

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
pub(super) struct EligibilityInput {
    pub media_format: Option<String>,
    pub media: Option<MediaInput>,
    #[serde(default)]
    pub content_redaction: Field<IgnoredAny>,
    #[serde(default)]
    pub toxicity_detection: Field<IgnoredAny>,
    pub model_settings: Option<LanguageModel>,
    pub language_id_settings: Option<BTreeMap<String, LanguageModel>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
pub(super) struct LanguageModel {
    #[serde(default)]
    pub language_model_name: Field<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
pub(super) struct MediaInput {
    #[serde(default)]
    pub media_file_uri: Field<String>,
    #[serde(default)]
    pub redacted_media_file_uri: Field<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
pub(super) struct StorageInput {
    pub media: Option<MediaInput>,
    #[serde(default)]
    pub output_bucket_name: Field<String>,
    #[serde(default)]
    pub data_access_role_arn: Field<IgnoredAny>,
    #[serde(default)]
    pub job_execution_settings: Field<IgnoredAny>,
}
