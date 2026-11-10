use serde::Deserialize;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
pub struct LoggingFlags {
    pub turn_off_message_logging: bool,
    pub message_logging: bool,
}

// LiteLLM's DataDogLLMObsLogger returns True here, so the base logger hands it the unredacted payload
pub const REDACTS_MESSAGES_ITSELF: bool = true;

// LiteLLM ORs the global flag, the deprecated per-logger flag and the request's own decision
#[must_use]
pub const fn payload_logging_is_off(flags: LoggingFlags, request_redacts: bool) -> bool {
    if flags.turn_off_message_logging || !flags.message_logging {
        return true;
    }
    request_redacts
}

mod tests;
