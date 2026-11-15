use serde::Serialize;

use super::{AUTO_LANGUAGE_CODE, ChirpProtocolError, ChirpSessionConfig};

#[derive(Serialize)]
struct ConfigureCommand<'config> {
    kind: CommandKind,
    model: &'config str,
    language_codes: [&'config str; 1],
    sample_rate_hertz: i64,
}

#[derive(Serialize)]
enum CommandKind {
    #[serde(rename = "configure")]
    Configure,
}

pub(super) fn configure_command(config: &ChirpSessionConfig) -> Result<String, ChirpProtocolError> {
    let command = ConfigureCommand {
        kind: CommandKind::Configure,
        model: &config.model,
        language_codes: [config.language.as_deref().unwrap_or(AUTO_LANGUAGE_CODE)],
        sample_rate_hertz: config.sample_rate,
    };
    serde_json::to_string(&command).map_err(ChirpProtocolError::Command)
}
