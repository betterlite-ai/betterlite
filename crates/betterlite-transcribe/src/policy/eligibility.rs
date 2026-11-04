use rust_decimal::Decimal;
use url::Url;

use super::inputs::EligibilityInput;
use crate::field::Field;

const PRICED_OPERATION: &str = "StartTranscriptionJob";
const PRICED_MODEL: &str = "transcribe/StartTranscriptionJob";
const MEASURABLE_FORMATS: [&str; 4] = ["flac", "mp3", "ogg", "wav"];
const UNPRICED_OPERATIONS: [&str; 3] = [
    "StartCallAnalyticsJob",
    "StartMedicalScribeJob",
    "StartMedicalTranscriptionJob",
];

#[must_use]
pub fn unpriceable_request_reason(
    operation: &str,
    request_body: &str,
    cost_per_second: Option<Decimal>,
) -> Option<String> {
    if UNPRICED_OPERATIONS.contains(&operation) {
        return Some(format!(
            "{operation} has no supported audio rate; only {PRICED_OPERATION} is priced"
        ));
    }
    if operation != PRICED_OPERATION {
        return None;
    }
    if cost_per_second.is_none() {
        return Some(format!(
            "{PRICED_MODEL} has no input_cost_per_second, so this job cannot be budgeted"
        ));
    }
    let body: EligibilityInput = match serde_json::from_str(request_body) {
        Ok(parsed) => parsed,
        Err(error) => return Some(format!("Invalid Transcribe request: {error}")),
    };
    let unsupported = unsupported_members(&body);
    if !unsupported.is_empty() {
        return Some(format!(
            "{PRICED_OPERATION} with {} adds an unpriced surcharge",
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

fn unsupported_members(body: &EligibilityInput) -> Vec<String> {
    let mut unsupported = Vec::new();
    if !body.content_redaction.is_absent() {
        unsupported.push("ContentRedaction".to_owned());
    }
    if !body.toxicity_detection.is_absent() {
        unsupported.push("ToxicityDetection".to_owned());
    }
    if body
        .model_settings
        .as_ref()
        .is_some_and(|settings| !settings.language_model_name.is_absent())
    {
        unsupported.push("ModelSettings.LanguageModelName".to_owned());
    }
    if let Some(languages) = &body.language_id_settings {
        for (language, settings) in languages {
            if !settings.language_model_name.is_absent() {
                unsupported.push(format!("LanguageIdSettings.{language}.LanguageModelName"));
            }
        }
    }
    unsupported
}

#[must_use]
pub fn requested_media_format(request_body: &str) -> Option<String> {
    let body: EligibilityInput = serde_json::from_str(request_body).ok()?;

    if let Some(format) = &body.media_format {
        return Some(format.to_lowercase());
    }
    let media = body.media.as_ref()?;
    let Field::Present(Some(uri)) = &media.media_file_uri else {
        return None;
    };
    let parsed = uri
        .contains("://")
        .then(|| Url::parse(uri))
        .transpose()
        .ok()?;
    let path = parsed.as_ref().map_or(uri.as_str(), Url::path);
    Some(path.rsplit_once('.')?.1.to_lowercase())
}
