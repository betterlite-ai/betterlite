use super::seconds::{COMPLETION_FALLBACK, Seconds};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TimeoutValue {
    Seconds(Seconds),
    Phases(TimeoutPhases),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimeoutPhases {
    pub connect: Option<Seconds>,
    pub read: Option<Seconds>,
    pub write: Option<Seconds>,
    pub pool: Option<Seconds>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct TimeoutKwargs {
    pub timeout: Option<Seconds>,
    pub request_timeout: Option<Seconds>,
}

#[must_use]
pub fn supports_httpx_timeout(provider: &str) -> bool {
    if provider == "openai" {
        return true;
    }
    provider == "azure" || provider == "bedrock"
}

// model_timeout, then kwargs["timeout"], then kwargs["request_timeout"], then the configured global or the 600 s fallback
#[must_use]
pub fn resolve(
    model_timeout: Option<TimeoutValue>,
    kwargs: &TimeoutKwargs,
    configured: Option<Seconds>,
    phases_supported: bool,
) -> TimeoutValue {
    let fallback = configured.unwrap_or(COMPLETION_FALLBACK);
    let resolved = model_timeout
        .or(kwargs.timeout)
        .or(kwargs.request_timeout)
        .unwrap_or(fallback);
    match resolved {
        TimeoutValue::Phases(phases) if !phases_supported => {
            TimeoutValue::Seconds(phases.read.unwrap_or(COMPLETION_FALLBACK))
        }
        resolved => resolved,
    }
}

impl TimeoutValue {
    // the shared transport enforces one total deadline, so every phase collapses to the read phase, else 600 s
    #[must_use]
    pub fn transport(self) -> Seconds {
        match self {
            Self::Seconds(seconds) => seconds,
            Self::Phases(phases) => phases.read.unwrap_or(COMPLETION_FALLBACK),
        }
    }
}
