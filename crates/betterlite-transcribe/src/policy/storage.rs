use std::collections::BTreeSet;

use betterlite_auth::auth_identity::records::Principal;
use betterlite_core::api::managed_resource_store::is_proxy_admin;
use serde::Deserialize;

use super::inputs::StorageInput;
use crate::field::Field;
use crate::records::TranscribeRefusal;

#[derive(Deserialize)]
struct BucketSettings {
    transcribe_media_buckets: Option<BTreeSet<String>>,
}

#[must_use]
pub fn transcribe_media_buckets(general_settings: &str) -> Option<BTreeSet<String>> {
    serde_json::from_str::<BucketSettings>(general_settings)
        .ok()?
        .transcribe_media_buckets
}

#[must_use]
pub fn s3_bucket_name(uri: &str) -> Option<&str> {
    let resource = uri.strip_prefix("s3://")?;
    let bucket = resource.split('/').next()?;
    (!bucket.is_empty()).then_some(bucket)
}

#[must_use]
pub fn transcribe_storage_refusal(
    request_body: &str,
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
    let body: StorageInput = match serde_json::from_str(request_body) {
        Ok(parsed) => parsed,
        Err(error) => {
            return Some(TranscribeRefusal {
                status_code: 400,
                detail: format!("Invalid Transcribe request: {error}"),
            });
        }
    };
    if !body.data_access_role_arn.is_absent() || !body.job_execution_settings.is_absent() {
        return Some(TranscribeRefusal {
            status_code: 403,
            detail: "DataAccessRoleArn and JobExecutionSettings may only be set by a proxy admin"
                .to_owned(),
        });
    }
    let offending = offending_location(&body, allowed)?;
    Some(TranscribeRefusal {
        status_code: 403,
        detail: format!(
            "{offending} must name a bucket in general_settings.transcribe_media_buckets"
        ),
    })
}

fn offending_location(body: &StorageInput, allowed: &BTreeSet<String>) -> Option<&'static str> {
    if let Some(media) = &body.media {
        for (member, uri) in [
            ("Media.MediaFileUri", &media.media_file_uri),
            ("Media.RedactedMediaFileUri", &media.redacted_media_file_uri),
        ] {
            if let Field::Present(present) = uri
                && !present
                    .as_deref()
                    .and_then(s3_bucket_name)
                    .is_some_and(|bucket| allowed.contains(bucket))
            {
                return Some(member);
            }
        }
    }
    if let Field::Present(output) = &body.output_bucket_name
        && !output
            .as_ref()
            .is_some_and(|bucket| allowed.contains(bucket))
    {
        return Some("OutputBucketName");
    }
    None
}
