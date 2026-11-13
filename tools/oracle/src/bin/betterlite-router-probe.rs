use std::io::{self, Read as _};
use std::process::ExitCode;

use betterlite_oracle::router_probes::{Reply, Request, execute};
use betterlite_oracle::routing_probe_ipc::{MAX_MESSAGE_BYTES, ProbeFailure, parse_message};

#[derive(Debug, thiserror::Error)]
enum ProbeIoError {
    #[error("cannot read the probe request: {0}")]
    Read(#[source] io::Error),
    #[error("cannot write the probe reply: {0}")]
    Write(#[source] serde_json::Error),
}

fn main() -> ExitCode {
    match answer_request() {
        Ok(()) => ExitCode::SUCCESS,
        Err(_error) => ExitCode::FAILURE,
    }
}

fn answer_request() -> Result<(), ProbeIoError> {
    let limit = u64::try_from(MAX_MESSAGE_BYTES.saturating_add(1)).unwrap_or(u64::MAX);
    let mut input = Vec::new();
    io::stdin()
        .lock()
        .take(limit)
        .read_to_end(&mut input)
        .map_err(ProbeIoError::Read)?;
    // a malformed request is a typed reply, so the oracle compares it with the Python probe's answer
    let reply = match parse_message::<Request>(&input) {
        Ok(request) => execute(&request),
        Err(error) => Reply::Error {
            kind: ProbeFailure::InvalidRequest,
            message: error.to_string(),
        },
    };
    let mut output = io::stdout().lock();
    serde_json::to_writer(&mut output, &reply).map_err(ProbeIoError::Write)?;
    Ok(())
}
