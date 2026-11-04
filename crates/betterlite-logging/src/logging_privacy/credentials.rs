use betterlite_models::decisions::Json;
use betterlite_redact::secret::Redactor;
use betterlite_redact::structured::{REDACTED, SensitiveDataMasker};

pub(super) fn scrub(value: &mut Json, redactor: &Redactor) {
    let mut walker = Scrubber {
        redactor,
        masker: SensitiveDataMasker::default(),
        remaining: 1_000_000,
    };
    walker.visit(value, None, 0);
}

struct Scrubber<'redactor> {
    redactor: &'redactor Redactor,
    masker: SensitiveDataMasker,
    remaining: usize,
}

impl Scrubber<'_> {
    fn visit(&mut self, value: &mut Json, key: Option<&str>, depth: usize) {
        let Some(remaining) = self.remaining.checked_sub(1) else {
            *value = Json::Text(REDACTED.to_owned());
            return;
        };
        self.remaining = remaining;
        if depth > 100
            || (!matches!(value, Json::Null)
                && key.is_some_and(|name| self.masker.is_sensitive_key(name, &[])))
        {
            *value = Json::Text(REDACTED.to_owned());
            return;
        }
        match value {
            Json::Object(fields) => {
                for (name, child) in fields {
                    self.visit(child, Some(name), depth.saturating_add(1));
                }
            }
            Json::List(values) => {
                for child in values {
                    self.visit(child, None, depth.saturating_add(1));
                }
            }
            Json::Text(text) => *text = self.redactor.redact_string(text),
            Json::Null | Json::Bool(_) | Json::Number(_) => {}
        }
    }
}
