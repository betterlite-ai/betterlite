pub mod child;
pub mod operation;
mod tests;

use std::time::Duration;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::OracleError;
use child::{Captured, ProbeCommand, run_bounded};
use operation::{ProbeOperation, ProbeOutput};

pub const MAX_MESSAGE_BYTES: usize = 1 << 20;
pub const OPERATION_DEADLINE: Duration = Duration::from_secs(30);
pub const BUILD_DEADLINE: Duration = Duration::from_secs(300);
const SAMPLE_LIMIT: u64 = 1 << 53;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProbeRequest {
    pub clock: ProbeClock,
    pub samples: Vec<ProbeSample>,
    pub operation: ProbeOperation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProbeClock {
    pub unix_millis: u64,
    pub monotonic_millis: u64,
}

// Python's random() draws 53 bits, so sample k stands for exactly k / 2^53 on both sides
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "u64")]
pub struct ProbeSample(u64);

impl TryFrom<u64> for ProbeSample {
    type Error = OracleError;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        if value < SAMPLE_LIMIT {
            Ok(Self(value))
        } else {
            Err(OracleError::ProbeSample(value))
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case", deny_unknown_fields)]
pub enum ProbeReply {
    Ok {
        output: ProbeOutput,
        warnings: Vec<String>,
    },
    Error {
        kind: ProbeFailure,
        message: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProbeFailure {
    InvalidRequest,
    UnknownOperation,
    SamplesExhausted,
}

#[derive(Debug)]
pub struct ProbeAnswer {
    pub reply: ProbeReply,
    pub stderr: Captured,
}

pub fn parse_message<Message>(bytes: &[u8]) -> Result<Message, OracleError>
where
    Message: DeserializeOwned,
{
    if bytes.len() > MAX_MESSAGE_BYTES {
        return Err(OracleError::ProbeTooLarge);
    }
    serde_json::from_slice(bytes).map_err(OracleError::ProbeMalformed)
}

pub async fn probe(
    command: &ProbeCommand,
    request: &ProbeRequest,
    deadline: Duration,
) -> Result<ProbeAnswer, OracleError> {
    let input = serde_json::to_vec(request).map_err(OracleError::Json)?;
    let output = run_bounded(command, &input, deadline).await?;
    if !output.status.success() {
        return Err(OracleError::ProbeExited {
            status: output.status,
            stderr: output.stderr.bytes,
        });
    }
    if output.stdout.truncated {
        return Err(OracleError::ProbeTooLarge);
    }
    Ok(ProbeAnswer {
        reply: parse_message(&output.stdout.bytes)?,
        stderr: output.stderr,
    })
}
