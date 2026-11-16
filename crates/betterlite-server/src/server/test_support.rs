use serde::Deserialize;
use std::io::{self, BufRead};
use thiserror::Error;

mod tests;

const LISTENING_MESSAGE: &str = "betterlite server listening";

#[derive(Debug, Error)]
pub enum ListeningPortError {
    #[error("failed to read the server output: {source}")]
    Read {
        #[source]
        source: io::Error,
    },
    #[error("the server output ended before its listening line")]
    Ended,
}

#[derive(Deserialize)]
struct LogLine {
    message: String,
    port: Option<u16>,
}

// the flat JSON log has other lines too, and a line that is not the listening one is not an error
#[must_use]
pub fn port_in_line(line: &str) -> Option<u16> {
    let event: LogLine = serde_json::from_str(line).ok()?;
    if event.message == LISTENING_MESSAGE {
        event.port
    } else {
        None
    }
}

// the caller keeps the reader open afterwards, because a closed stderr pipe would break the server's later log writes
pub fn listening_port<Output>(output: &mut Output) -> Result<u16, ListeningPortError>
where
    Output: BufRead,
{
    for read in output.lines() {
        let line = read.map_err(|source| ListeningPortError::Read { source })?;
        if let Some(port) = port_in_line(&line) {
            return Ok(port);
        }
    }
    Err(ListeningPortError::Ended)
}
