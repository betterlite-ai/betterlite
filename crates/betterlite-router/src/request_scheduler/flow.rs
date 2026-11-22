#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Priority {
    High,
    Medium,
    Low,
}
// DefaultPriorities in litellm/types/scheduler.py: a lower number is served first
impl Priority {
    #[must_use]
    pub const fn value(self) -> u8 {
        match self {
            Self::High => 0,
            Self::Medium => 128,
            Self::Low => 255,
        }
    }
}
// Scheduler.add_request's FlowItem: the request's priority, its id and the model group whose queue it joins
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FlowItem {
    pub priority: u8,
    pub request_id: String,
    pub model: String,
}
