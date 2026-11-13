use betterlite_redact::secret::Redactor;

use super::{ErrorFacts, class::Class};
use crate::dispatch::{DispatchInput, RawException, dispatch};
use crate::facts::{MAX_BODY_BYTES, bounded_text};
use crate::parse::extract_facts;

#[must_use]
pub fn provider_facts(status: u16, body: &[u8], redactor: &Redactor) -> ErrorFacts {
    let bounded = body.get(..MAX_BODY_BYTES).unwrap_or(body);
    let extracted = extract_facts(bounded, Some(status));
    let message = extracted
        .message
        .unwrap_or_else(|| bounded_text(str::from_utf8(bounded).unwrap_or_default()));
    let input = RawException {
        model: String::new(),
        custom_llm_provider: "openai".to_owned(),
        error_str: message.clone(),
        status_code: Some(status),
        status_code_is_synthesized: false,
        message: Some(message.clone()),
        body: None,
        exception_type: String::new(),
    };
    let class = dispatch(&DispatchInput::Raw(Box::new(input)), redactor);
    let metadata = class.metadata();
    ErrorFacts {
        proxy_type: None,
        message: redactor.redact_internal_details(&message),
        class: Class::from(&class),
        status_code: Some(metadata.status_code),
        rate_limit_type: metadata.rate_limit_dimension,
    }
}

mod tests;
