use betterlite_auth::auth_identity::records::Principal;
use betterlite_core::api::managed_resource_store::{
    get_primary_owner_scope, is_proxy_admin, user_can_access_resource_owner,
};

use crate::field::Field;
use crate::records::{
    GetTranscriptionJobRequest, JobLookup, TranscribeRefusal, TranscriptionJobResponse,
};
use crate::start_request::{OwnedStartRequest, PassthroughFields, StartTag};

const OWNER_TAG: &str = "litellm-owner";

pub fn transcribe_owned_start_request(
    request_body: &str,
    principal: Option<&Principal>,
) -> Result<OwnedStartRequest, TranscribeRefusal> {
    let owner = get_primary_owner_scope(principal).ok_or_else(|| TranscribeRefusal {
        status_code: 400,
        detail: "The calling key has no identity to record as job owner".to_owned(),
    })?;
    let mut body: OwnedStartRequest =
        serde_json::from_str(request_body).map_err(|error| TranscribeRefusal {
            status_code: 400,
            detail: format!("Invalid Transcribe request or Tags: {error}"),
        })?;
    for tag in &body.tags {
        validate_tag(tag)?;
    }
    body.tags.push(StartTag {
        key: Field::Present(Some(OWNER_TAG.to_owned())),
        value: Field::Present(Some(owner.0)),
        passthrough: PassthroughFields::default(),
    });
    Ok(body)
}

fn validate_tag(tag: &StartTag) -> Result<(), TranscribeRefusal> {
    if tag.key.is_absent() || matches!(&tag.key, Field::Present(Some(key)) if key.is_empty()) {
        return Err(TranscribeRefusal {
            status_code: 400,
            detail: "Each tag must have a non-empty Key".to_owned(),
        });
    }
    if matches!(&tag.key, Field::Present(Some(key)) if key == OWNER_TAG) {
        return Err(TranscribeRefusal {
            status_code: 400,
            detail: "The litellm-owner tag is assigned by the proxy".to_owned(),
        });
    }
    Ok(())
}

fn job_response_access_refusal(
    response: &TranscriptionJobResponse,
    principal: Option<&Principal>,
) -> Option<TranscribeRefusal> {
    let owner = response.transcription_job.as_ref().and_then(|job| {
        job.tags
            .iter()
            .find(|tag| tag.key.as_deref() == Some(OWNER_TAG))
            .and_then(|tag| tag.value.as_deref())
    });
    let allowed = owner.is_some_and(|scope| user_can_access_resource_owner(Some(scope), principal));
    (!allowed).then(|| TranscribeRefusal {
        status_code: 404,
        detail: "No transcription job was started through this proxy by the calling key".to_owned(),
    })
}

pub async fn transcribe_job_access_refusal(
    job_name: &str,
    principal: Option<&Principal>,
    lookup: &dyn JobLookup,
) -> Option<TranscribeRefusal> {
    if is_proxy_admin(principal) {
        return None;
    }
    let Ok(name) = serde_json::from_str(job_name) else {
        return Some(TranscribeRefusal {
            status_code: 400,
            detail: "TranscriptionJobName must be a string".to_owned(),
        });
    };
    let request = GetTranscriptionJobRequest {
        transcription_job_name: name,
    };
    let Ok(response) = lookup.get_transcription_job(&request).await else {
        return Some(TranscribeRefusal {
            status_code: 404,
            detail: "No accessible transcription job was found".to_owned(),
        });
    };
    job_response_access_refusal(&response, principal)
}
