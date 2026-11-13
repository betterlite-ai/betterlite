use betterlite_redact::error::RedactionError;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};

pub(crate) struct TextRedactor {
    redactor: Redactor,
}

impl TextRedactor {
    pub(crate) fn from_defaults() -> Result<Self, RedactionError> {
        let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)?;
        Ok(Self { redactor })
    }

    pub(crate) fn redact(&self, value: &str) -> String {
        self.redactor.redact_string(value)
    }
}
