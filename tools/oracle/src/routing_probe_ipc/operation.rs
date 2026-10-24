use serde::{Deserialize, Serialize};

use super::{ProbeClock, ProbeSample};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProbeOperation {
    // Seeded for the IPC tooling tests; routing operations join this enum in later tasks
    Echo(EchoInput),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EchoInput {
    pub text: String,
    pub draws: u32,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProbeOutput {
    Echo(EchoOutput),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EchoOutput {
    pub text: String,
    pub clock: ProbeClock,
    pub samples: Vec<ProbeSample>,
}
