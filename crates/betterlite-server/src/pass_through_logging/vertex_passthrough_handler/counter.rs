use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};

use crate::pass_through_logging::gemini_passthrough_logging::counter::CollectedCounter;

// The partner transforms count reasoning tokens with LiteLLM's passthrough tokenizer and redact their debug log.
pub(super) struct PartnerTools {
    pub(super) counter: CollectedCounter,
    pub(super) redactor: Redactor,
}

pub(super) fn partner_tools() -> Option<PartnerTools> {
    let Some(counter) = CollectedCounter::bundled() else {
        tracing::error!("Vertex passthrough logging: the bundled cl100k tokenizer is missing");
        return None;
    };
    match Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH) {
        Ok(redactor) => Some(PartnerTools { counter, redactor }),
        Err(error) => {
            tracing::error!(
                "Vertex passthrough logging: the log redactor failed to build: {error}"
            );
            None
        }
    }
}
