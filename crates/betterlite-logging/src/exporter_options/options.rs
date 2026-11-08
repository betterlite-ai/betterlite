use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(from = "RawLoggerOptions")]
pub struct CustomLoggerOptions {
    pub turn_off_message_logging: bool,
    pub message_logging: bool,
}

#[derive(Debug, Deserialize)]
struct RawLoggerOptions {
    #[serde(default)]
    turn_off_message_logging: Option<bool>,
    #[serde(default)]
    message_logging: Option<bool>,
}

impl From<RawLoggerOptions> for CustomLoggerOptions {
    fn from(raw: RawLoggerOptions) -> Self {
        Self {
            turn_off_message_logging: raw.turn_off_message_logging.unwrap_or(false),
            message_logging: raw.message_logging.unwrap_or(true),
        }
    }
}

impl Default for CustomLoggerOptions {
    fn default() -> Self {
        Self {
            turn_off_message_logging: false,
            message_logging: true,
        }
    }
}
