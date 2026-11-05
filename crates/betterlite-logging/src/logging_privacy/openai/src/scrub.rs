use super::SnapshotError;
use betterlite_logging::logging_privacy::credentials::scrub_value;
use betterlite_openai::stream::chunk::Logprobs;
use betterlite_redact::secret::Redactor;
use serde::{Serialize, de::DeserializeOwned};

pub(super) fn passthrough<T>(fields: &mut T, redactor: &Redactor) -> Result<(), SnapshotError>
where
    T: Serialize + DeserializeOwned,
{
    let mut value = serde_json::to_value(&*fields)?;
    scrub_value(&mut value, redactor);
    *fields = serde_json::from_value(value)?;
    Ok(())
}

pub(super) fn logprobs(probabilities: &mut Logprobs, redactor: &Redactor) {
    for token in probabilities
        .content
        .iter_mut()
        .chain(probabilities.refusal.iter_mut())
        .flatten()
    {
        let safe = redactor.redact_string(&token.token);
        if safe != token.token {
            token.bytes = None;
        }
        token.token = safe;
        for alternative in &mut token.top_logprobs {
            let sanitized = redactor.redact_string(&alternative.token);
            if sanitized != alternative.token {
                alternative.bytes = None;
            }
            alternative.token = sanitized;
        }
    }
}
