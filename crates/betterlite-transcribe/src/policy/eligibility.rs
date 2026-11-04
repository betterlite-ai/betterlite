use betterlite_auth::auth_identity::records::Principal;
use betterlite_core::api::managed_resource_store::is_proxy_admin;
use rust_decimal::Decimal;
use serde_json::Value;

use crate::records::TranscribeRefusal;

const PRICED_OPERATION: &str = "StartTranscriptionJob";
const PRICED_MODEL: &str = "transcribe/StartTranscriptionJob";
const MEASURABLE_FORMATS: [&str; 4] = ["flac", "mp3", "ogg", "wav"];
const SURCHARGES: [&str; 2] = ["ContentRedaction", "ToxicityDetection"];
const UNPRICED_OPERATIONS: [&str; 3] = [
    "StartCallAnalyticsJob",
    "StartMedicalScribeJob",
    "StartMedicalTranscriptionJob",
];
const OWNED_JOB_OPERATIONS: [&str; 2] = ["GetTranscriptionJob", "DeleteTranscriptionJob"];

#[must_use]
pub fn unpriceable_request_reason(
    operation: &str,
    request_body: &Value,
    cost_per_second: Option<Decimal>,
) -> Option<String> {
    if UNPRICED_OPERATIONS.contains(&operation) {
        return Some(format!(
            "{operation} is billed per second of audio at a rate LiteLLM does not price yet; only {PRICED_OPERATION} is supported"
        ));
    }
    if operation != PRICED_OPERATION {
        return None;
    }
    let Some(rate) = cost_per_second else {
        return Some(format!(
            "{PRICED_MODEL} has no input_cost_per_second, so this job cannot be budgeted"
        ));
    };
    if rate.is_sign_negative() {
        return Some(format!(
            "{PRICED_MODEL} has an invalid input_cost_per_second"
        ));
    }
    let unsupported = unsupported_members(request_body);
    if !unsupported.is_empty() {
        return Some(format!(
            "{PRICED_OPERATION} with {} adds a surcharge that LiteLLM does not price",
            unsupported.join(", ")
        ));
    }
    if !requested_media_format(request_body)
        .as_deref()
        .is_some_and(|format| MEASURABLE_FORMATS.contains(&format))
    {
        return Some(format!(
            "Media format must be measurable as one of {}",
            MEASURABLE_FORMATS.join(", ")
        ));
    }
    None
}

fn unsupported_members(request_body: &Value) -> Vec<String> {
    let body = request_body.as_object();
    let mut unsupported = SURCHARGES
        .iter()
        .filter(|member| body.is_some_and(|body| body.contains_key(**member)))
        .map(|member| (*member).to_owned())
        .collect::<Vec<_>>();
    if body
        .and_then(|body| body.get("ModelSettings"))
        .and_then(Value::as_object)
        .is_some_and(|settings| settings.contains_key("LanguageModelName"))
    {
        unsupported.push("ModelSettings.LanguageModelName".to_owned());
    }
    if let Some(languages) = body
        .and_then(|body| body.get("LanguageIdSettings"))
        .and_then(Value::as_object)
    {
        for (language, settings) in languages {
            if settings
                .as_object()
                .is_some_and(|settings| settings.contains_key("LanguageModelName"))
            {
                unsupported.push(format!("LanguageIdSettings.{language}.LanguageModelName"));
            }
        }
    }
    unsupported
}

#[must_use]
pub fn requested_media_format(request_body: &Value) -> Option<String> {
    if let Some(format) = request_body.get("MediaFormat").and_then(Value::as_str) {
        return Some(format.to_lowercase());
    }
    media_uri_extension(request_body.get("Media")?.get("MediaFileUri")?.as_str()?)
}

fn media_uri_extension(uri: &str) -> Option<String> {
    let path = if let Some((_, resource)) = uri.rsplit_once("://") {
        resource.split_once('/')?.1
    } else {
        uri
    };
    let path = path.split(['?', '#']).next()?;
    Some(path.rsplit('/').next()?.rsplit_once('.')?.1.to_lowercase())
}

#[must_use]
pub fn transcribe_admin_only_refusal(
    operation: &str,
    principal: Option<&Principal>,
) -> Option<TranscribeRefusal> {
    if operation == PRICED_OPERATION
        || OWNED_JOB_OPERATIONS.contains(&operation)
        || is_proxy_admin(principal)
    {
        return None;
    }
    Some(TranscribeRefusal {
        status_code: 403,
        detail: format!(
            "{operation} can access every Transcribe resource in the AWS account; only a proxy admin may call it"
        ),
    })
}
