use std::iter;

use betterlite_models::decisions::Json;
use betterlite_redact::secret::Redactor;
use betterlite_redact::structured::Payload;
use betterlite_redact::structured::REDACTED;
use betterlite_redact::structured::redact::redact_credentials_in_payload;

pub(super) fn scrub(value: &mut Json, redactor: &Redactor) {
    let Ok(mut encoded) = serde_json::to_value(&*value) else {
        *value = Json::Text(REDACTED.to_owned());
        return;
    };
    scrub_value(&mut encoded, redactor);
    *value = serde_json::from_value(encoded).unwrap_or_else(|_| Json::Text(REDACTED.to_owned()));
}

pub fn scrub_value(value: &mut Payload, redactor: &Redactor) {
    let wrapper = iter::once(("payload".to_owned(), value.take())).collect();
    let mut safe = redact_credentials_in_payload(&wrapper);
    scrub_text(&mut safe, redactor);
    *value = match safe {
        Payload::Object(mut fields) => fields
            .remove("payload")
            .unwrap_or_else(|| Payload::String(REDACTED.to_owned())),
        other @ (Payload::Null
        | Payload::Bool(_)
        | Payload::Number(_)
        | Payload::String(_)
        | Payload::Array(_)) => other,
    };
}

fn scrub_text(value: &mut Payload, redactor: &Redactor) {
    match value {
        Payload::String(text) => *text = redactor.redact_string(text),
        Payload::Array(values) => values
            .iter_mut()
            .for_each(|child| scrub_text(child, redactor)),
        Payload::Object(fields) => fields
            .values_mut()
            .for_each(|child| scrub_text(child, redactor)),
        Payload::Null | Payload::Bool(_) | Payload::Number(_) => {}
    }
}
