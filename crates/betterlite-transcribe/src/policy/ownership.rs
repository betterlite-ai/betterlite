use betterlite_auth::auth_identity::records::Principal;
use betterlite_core::api::managed_resource_store::{
    get_primary_owner_scope, is_proxy_admin, user_can_access_resource_owner,
};
use serde_json::Value;

use crate::records::{
    GetTranscriptionJobRequest, JobLookup, TranscribeRefusal, TranscriptionJobResponse,
};

const OWNER_TAG: &str = "litellm-owner";

#[must_use]
pub fn transcribe_owned_start_request(
    request_body: &Value,
    principal: Option<&Principal>,
) -> Result<Value, TranscribeRefusal> {
    let owner = primary_owner(principal)?;
    let body = request_body.as_object().ok_or_else(|| TranscribeRefusal {
        status_code: 400,
        detail: "Transcribe request must be an object".to_owned(),
    })?;
    let mut tags = parse_tags(body.get("Tags"))?;
    tags.push(serde_json::json!({"Key": OWNER_TAG, "Value": owner}));
    let mut updated = body.clone();
    updated.insert("Tags".to_owned(), Value::Array(tags));
    Ok(Value::Object(updated))
}

fn primary_owner(principal: Option<&Principal>) -> Result<String, TranscribeRefusal> {
    principal
        .and_then(|principal| get_primary_owner_scope(Some(principal)))
        .map(|owner| owner.0)
        .ok_or_else(|| TranscribeRefusal {
            status_code: 400,
            detail: "The calling key has no identity to record as job owner".to_owned(),
        })
}

fn parse_tags(tags: Option<&Value>) -> Result<Vec<Value>, TranscribeRefusal> {
    let Some(tags) = tags else {
        return Ok(Vec::new());
    };
    let Some(tags) = tags.as_array() else {
        return Err(TranscribeRefusal {
            status_code: 400,
            detail: "Tags must be a list of objects with Key and Value members".to_owned(),
        });
    };
    tags.iter().map(parse_tag).collect()
}

fn parse_tag(tag: &Value) -> Result<Value, TranscribeRefusal> {
    let Some(tag) = tag.as_object() else {
        return Err(TranscribeRefusal {
            status_code: 400,
            detail: "Tags must be a list of objects with Key and Value members".to_owned(),
        });
    };
    let key = tag.get("Key");
    if key.is_none()
        || key.is_some_and(|key| !key.is_null() && (!key.is_string() || key.as_str() == Some("")))
    {
        return Err(TranscribeRefusal {
            status_code: 400,
            detail: "Each tag must have a non-empty string Key".to_owned(),
        });
    }
    if key.and_then(Value::as_str) == Some(OWNER_TAG) {
        return Err(TranscribeRefusal {
            status_code: 400,
            detail: "The litellm-owner tag is assigned by LiteLLM".to_owned(),
        });
    }
    for member in ["Key", "Value"] {
        if tag
            .get(member)
            .is_some_and(|value| !value.is_null() && !value.is_string())
        {
            return Err(TranscribeRefusal {
                status_code: 400,
                detail: format!("Tag {member} must be a string"),
            });
        }
    }
    Ok(Value::Object(tag.clone()))
}

#[must_use]
pub fn job_access_refusal(
    job_response: &Value,
    principal: Option<&Principal>,
) -> Option<TranscribeRefusal> {
    if is_proxy_admin(principal) {
        return None;
    }
    let response: Result<TranscriptionJobResponse, _> =
        serde_json::from_value(job_response.clone());
    match response {
        Ok(response) => job_response_access_refusal(&response, principal),
        Err(_) => Some(TranscribeRefusal {
            status_code: 404,
            detail: "No transcription job was started through this proxy by the calling key"
                .to_owned(),
        }),
    }
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
    let allowed = owner.is_some_and(|owner| user_can_access_resource_owner(Some(owner), principal));
    (!allowed).then(|| TranscribeRefusal {
        status_code: 404,
        detail: "No transcription job was started through this proxy by the calling key".to_owned(),
    })
}

pub async fn transcribe_job_access_refusal(
    job_name: &Value,
    principal: Option<&Principal>,
    lookup: &dyn JobLookup,
) -> Option<TranscribeRefusal> {
    if is_proxy_admin(principal) {
        return None;
    }
    let Some(job_name) = job_name.as_str() else {
        return Some(TranscribeRefusal {
            status_code: 400,
            detail: "TranscriptionJobName must be a string".to_owned(),
        });
    };
    let request = GetTranscriptionJobRequest {
        transcription_job_name: job_name.to_owned(),
    };
    let Ok(response) = lookup.get_transcription_job(&request).await else {
        return Some(TranscribeRefusal {
            status_code: 404,
            detail: "No transcription job was started through this proxy by the calling key"
                .to_owned(),
        });
    };
    job_response_access_refusal(&response, principal)
}
