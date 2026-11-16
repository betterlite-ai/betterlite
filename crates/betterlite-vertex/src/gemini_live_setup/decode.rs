use serde::Deserialize;
use serde_json::Deserializer;

use super::error::LiveSetupError;
use super::wire::LiveSetup;

#[derive(Debug, Deserialize)]
struct SetupEnvelope {
    setup: Option<LiveSetup>,
}

pub fn decode_gemini_live_setup(request: &str) -> Result<LiveSetup, LiveSetupError> {
    let mut decoder = Deserializer::from_str(request);
    let envelope: SetupEnvelope =
        serde_path_to_error::deserialize(&mut decoder).map_err(decode_error)?;
    decoder.end().map_err(LiveSetupError::InvalidJson)?;
    Ok(envelope.setup.unwrap_or_default())
}

fn decode_error(error: serde_path_to_error::Error<serde_json::Error>) -> LiveSetupError {
    let path = error.path().to_string();
    let source = error.into_inner();
    if source.is_data() {
        return LiveSetupError::InvalidField { path, source };
    }
    LiveSetupError::InvalidJson(source)
}
