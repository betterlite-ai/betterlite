use betterlite_models::decisions::Json;
use betterlite_redact::secret::Redactor;
use serde::Serialize;

use super::request::Decision;
use super::{CallbackMetadataError, callback_snapshot, credentials, redact_user_api_key_info};

pub fn snapshot<Payload: Serialize>(
    source: &Payload,
    metadata: &Json,
    decision: Decision,
    redactor: &Redactor,
) -> Result<(Json, Json), CallbackMetadataError> {
    let encoded = serde_json::to_vec(source)?;
    let parsed: Json = serde_json::from_slice(&encoded)?;
    let payload = callback_snapshot(parsed, decision, redactor);
    let mut safe_metadata = redact_user_api_key_info(metadata, decision.key_info);
    credentials::scrub(&mut safe_metadata, redactor);
    Ok((payload, safe_metadata))
}
