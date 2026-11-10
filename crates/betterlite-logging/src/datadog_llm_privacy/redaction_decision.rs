use serde::Deserialize;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
pub struct LoggingFlags {
    pub turn_off_message_logging: bool,
    pub message_logging: bool,
}

// LiteLLM's DataDogLLMObsLogger returns True here, so the base logger hands it the unredacted payload
pub const REDACTS_MESSAGES_ITSELF: bool = true;

mod tests;
