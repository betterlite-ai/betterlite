use betterlite_error::proxy_exception::{ExceptionCode, ProxyException, ProxyExceptionInit};

const UNAUTHORIZED: &str = "401";
const UNAUTHORIZED_STATUS: u16 = 401;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorClass {
    Http,
    Proxy,
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Marker {
    Absent,
    NotTrue,
    ExactTrue,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ErrorFacts<'facts> {
    pub class: ErrorClass,
    pub code: Option<&'facts str>,
    pub status_code: Option<u16>,
    pub marker: Marker,
}

impl<'facts> ErrorFacts<'facts> {
    #[must_use]
    pub fn proxy(exception: &'facts ProxyException) -> Self {
        Self {
            class: ErrorClass::Proxy,
            code: Some(exception.code.as_str()),
            status_code: None,
            marker: Marker::Absent,
        }
    }

    // mark_invalid_virtual_key_error sets the marker on the copy it returns, so the outcome carries it
    #[must_use]
    pub fn marked(exception: &'facts ProxyException) -> Self {
        Self {
            marker: Marker::ExactTrue,
            ..Self::proxy(exception)
        }
    }
}

#[must_use]
pub fn is_invalid_virtual_key_error(facts: &ErrorFacts<'_>) -> bool {
    if matches!(facts.class, ErrorClass::Other) {
        return false;
    }
    let matches_status = match (facts.code, facts.status_code) {
        (Some(code), _) => code == UNAUTHORIZED,
        (None, Some(status)) => status == UNAUTHORIZED_STATUS,
        (None, None) => false,
    };
    matches_status && matches!(facts.marker, Marker::ExactTrue)
}

#[derive(Debug, PartialEq, Eq)]
pub enum MarkOutcome {
    Same,
    Marked(Box<ProxyException>),
}

impl MarkOutcome {
    #[must_use]
    pub const fn marker(&self) -> Marker {
        match self {
            Self::Same => Marker::Absent,
            Self::Marked(_) => Marker::ExactTrue,
        }
    }
}

#[must_use]
pub fn mark_invalid_virtual_key_error(
    exception: &ProxyException,
    is_invalid_virtual_key: bool,
) -> MarkOutcome {
    if !is_invalid_virtual_key || exception.code != UNAUTHORIZED {
        return MarkOutcome::Same;
    }
    MarkOutcome::Marked(Box::new(ProxyException::new(ProxyExceptionInit {
        message: exception.message.clone(),
        error_type: exception.error_type.clone(),
        param: exception.param.clone(),
        code: ExceptionCode::Text(exception.code.clone()),
        headers: exception.headers.clone(),
        openai_code: exception.openai_code.clone(),
        provider_specific_fields: exception.provider_specific_fields.clone(),
    })))
}
