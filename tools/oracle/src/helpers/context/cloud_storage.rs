use betterlite_context::cloud_storage::{
    self, Encoding,
    policy::{self, LegacyCloudSetting, TrustedCloudCredentials},
};
use serde::{Deserialize, de::IgnoredAny};
use serde_json::json;
#[derive(Deserialize)]
#[serde(untagged)]
pub(crate) enum Scalar {
    Text(String),
    Boolean(bool),
    Other(IgnoredAny),
}
#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    CloudUriDetection {
        value: Scalar,
    },
    ObjectComponentSanitization {
        value: Option<Scalar>,
        fallback: Option<String>,
    },
    ObjectPathSanitization {
        value: Option<Scalar>,
        fallback: Option<String>,
    },
    ConfiguredBucketSplit {
        value: Scalar,
    },
    ObjectPathValidation {
        value: String,
    },
    CloudObjectEncoding {
        value: String,
    },
    LegacyTrustedPolicy {
        value: Scalar,
        #[serde(default)]
        trusted: bool,
    },
    #[serde(alias = "legacy_prefix_boundary", alias = "encoded_traversal_order")]
    ManagedIdAuthorization {
        file_id: String,
        scheme: String,
        bucket: String,
        #[serde(default)]
        prefixes: Vec<String>,
        #[serde(default)]
        legacy: bool,
    },
}
pub(crate) struct Op;
impl crate::helpers::Helper for Op {
    type Input = Input;
    fn run(input: &Input) -> crate::helpers::HelperResult {
        match input {
            Input::CloudUriDetection { value } => Ok(
                json!({"value": cloud_storage::is_managed_cloud_storage_uri(text(Some(value)).unwrap_or(""))}),
            ),
            Input::ObjectComponentSanitization { value, fallback } => Ok(
                json!({"value": cloud_storage::sanitize_component(text(value.as_ref()), fallback.as_deref().unwrap_or("file"))}),
            ),
            Input::ObjectPathSanitization { value, fallback } => Ok(
                json!({"value": cloud_storage::sanitize_path(text(value.as_ref()), fallback.as_deref().unwrap_or("file")), "name": cloud_storage::build_object_name("raw/", text(value.as_ref()), fallback.as_deref().unwrap_or("file"), uuid::Uuid::nil())}),
            ),
            Input::ConfiguredBucketSplit { value } => {
                cloud_storage::split_bucket_name(text(Some(value)).unwrap_or(""))
                    .map(|(bucket, prefix)| json!({"bucket": bucket, "prefix": prefix}))
                    .map_err(Box::from)
            }
            Input::ObjectPathValidation { value } => {
                cloud_storage::validate_cloud_object_path(value)
                    .map(|()| json!({"ok": true}))
                    .map_err(Box::from)
            }
            Input::CloudObjectEncoding { value } => Ok(
                json!({"gcs": cloud_storage::encode_object_name(value, Encoding::Gcs), "s3": cloud_storage::encode_object_name(value, Encoding::S3)}),
            ),
            Input::LegacyTrustedPolicy { value, trusted } => Ok(legacy(value, *trusted)),
            Input::ManagedIdAuthorization {
                file_id,
                scheme,
                bucket,
                prefixes,
                legacy,
            } => {
                let allowed: Vec<&str> = prefixes.iter().map(String::as_str).collect();
                cloud_storage::decode_cloud_object_reference(
                    file_id, scheme, bucket, &allowed, *legacy,
                )
                .map(|(found, object)| json!({"bucket": found, "object": object}))
                .map_err(Box::from)
            }
        }
    }
}
fn legacy(value: &Scalar, trusted: bool) -> serde_json::Value {
    let setting = match value {
        Scalar::Text(setting) => LegacyCloudSetting::Text(setting.clone()),
        Scalar::Boolean(setting) => LegacyCloudSetting::Boolean(*setting),
        Scalar::Other(_) => LegacyCloudSetting::Other,
    };
    let credentials = trusted.then(|| TrustedCloudCredentials::from(setting));
    json!({"value": policy::allow_legacy(credentials.as_ref())})
}
fn text(value: Option<&Scalar>) -> Option<&str> {
    match value {
        Some(Scalar::Text(text)) => Some(text),
        Some(Scalar::Boolean(_) | Scalar::Other(_)) | None => None,
    }
}
impl From<cloud_storage::CloudStorageError> for Box<crate::helpers::HelperError> {
    fn from(value: cloud_storage::CloudStorageError) -> Self {
        Self::new(crate::helpers::HelperError {
            class: "ValueError".to_owned(),
            status: None,
            error_type: Some(None),
            param: Some(None),
            code: Some(None),
            message: value.to_string(),
        })
    }
}
mod tests;
