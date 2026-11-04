use std::collections::BTreeSet;

use betterlite_auth::auth_identity::records::Principal;
use betterlite_core::api::managed_resource_store::is_proxy_admin;
use serde_json::Value;

use crate::records::TranscribeRefusal;

const ROLE_MEMBERS: [&str; 2] = ["DataAccessRoleArn", "JobExecutionSettings"];
const MEDIA_URI_MEMBERS: [&str; 2] = ["MediaFileUri", "RedactedMediaFileUri"];

#[must_use]
pub fn transcribe_media_buckets(general_settings: &Value) -> Option<BTreeSet<String>> {
    general_settings
        .get("transcribe_media_buckets")?
        .as_array()?
        .iter()
        .map(|bucket| bucket.as_str().map(str::to_owned))
        .collect()
}

#[must_use]
pub fn s3_bucket_name(uri: &Value) -> Option<&str> {
    let uri = uri.as_str()?.strip_prefix("s3://")?;
    let bucket = uri.split('/').next()?;
    (!bucket.is_empty()).then_some(bucket)
}

#[must_use]
pub fn transcribe_storage_refusal(
    request_body: &Value,
    allowed_buckets: Option<&BTreeSet<String>>,
    principal: Option<&Principal>,
) -> Option<TranscribeRefusal> {
    if is_proxy_admin(principal) {
        return None;
    }
    let Some(allowed) = allowed_buckets else {
        return Some(TranscribeRefusal {
            status_code: 403,
            detail: "general_settings.transcribe_media_buckets must list permitted S3 buckets"
                .to_owned(),
        });
    };
    let Some(body) = request_body.as_object() else {
        return Some(TranscribeRefusal {
            status_code: 400,
            detail: "Transcribe request must be an object".to_owned(),
        });
    };
    if let Some(role) = ROLE_MEMBERS
        .iter()
        .find(|member| body.contains_key(**member))
    {
        return Some(TranscribeRefusal {
            status_code: 403,
            detail: format!("{role} may only be set by a proxy admin"),
        });
    }
    let Some(offending) = offending_location(body, allowed) else {
        return None;
    };
    Some(TranscribeRefusal {
        status_code: 403,
        detail: format!(
            "{offending} must name a bucket in general_settings.transcribe_media_buckets"
        ),
    })
}

fn offending_location(
    body: &serde_json::Map<String, Value>,
    allowed: &BTreeSet<String>,
) -> Option<String> {
    if let Some(media) = body.get("Media").and_then(Value::as_object) {
        for member in MEDIA_URI_MEMBERS {
            if let Some(uri) = media.get(member) {
                if !s3_bucket_name(uri).is_some_and(|bucket| allowed.contains(bucket)) {
                    return Some(format!("Media.{member}"));
                }
            }
        }
    }
    if let Some(output) = body.get("OutputBucketName") {
        if !output
            .as_str()
            .is_some_and(|bucket| allowed.contains(bucket))
        {
            return Some("OutputBucketName".to_owned());
        }
    }
    None
}
