#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CallbackPhase {
    Input,
    Success,
    Failure,
    Stream,
    Service,
}

impl CallbackPhase {
    pub const ALL: [Self; 5] = [
        Self::Input,
        Self::Success,
        Self::Failure,
        Self::Stream,
        Self::Service,
    ];

    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Input => "input",
            Self::Success => "success",
            Self::Failure => "failure",
            Self::Stream => "stream",
            Self::Service => "service",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CallbackList {
    Callbacks,
    Success,
    Failure,
    Input,
    Service,
}

impl CallbackList {
    pub const ALL: [Self; 5] = [
        Self::Callbacks,
        Self::Success,
        Self::Failure,
        Self::Input,
        Self::Service,
    ];

    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Callbacks => "callbacks",
            Self::Success => "success_callback",
            Self::Failure => "failure_callback",
            Self::Input => "input_callback",
            Self::Service => "service_callback",
        }
    }

    // LiteLLM dispatches a streaming response's events through log_stream_event instead of log_success_event
    #[must_use]
    pub const fn phases(self) -> &'static [CallbackPhase] {
        match self {
            Self::Callbacks => &[
                CallbackPhase::Success,
                CallbackPhase::Failure,
                CallbackPhase::Stream,
            ],
            Self::Success => &[CallbackPhase::Success, CallbackPhase::Stream],
            Self::Failure => &[CallbackPhase::Failure],
            Self::Input => &[CallbackPhase::Input],
            Self::Service => &[CallbackPhase::Service],
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CallbackSelection {
    pub success: Vec<String>,
    pub failure: Vec<String>,
    pub success_and_failure: Vec<String>,
}
