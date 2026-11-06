#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, strum::Display)]
#[strum(serialize_all = "snake_case")]
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
    // function_setup copies callbacks entries into input_callback, and streams reach success callbacks
    #[must_use]
    pub const fn phases(self) -> &'static [CallbackPhase] {
        match self {
            Self::Callbacks => &[
                CallbackPhase::Input,
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
