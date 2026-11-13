use std::collections::BTreeMap;
use std::mem;
use std::pin::pin;
use std::string::FromUtf8Error;

use futures_util::{Stream, StreamExt as _};
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use thiserror::Error;

pub const SANDBOX_MAX_OUTPUT_BYTES: usize = 10 * 1024 * 1024;
const CODE_EXECUTION_OBJECT: &str = "code_execution";

#[derive(Debug, Clone, Default)]
pub struct HiddenParams {
    pub envd_access_token: Option<SecretString>,
    pub traffic_access_token: Option<SecretString>,
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ContainerHandle {
    pub id: String,
    pub provider: String,
    pub domain: Option<String>,
    #[serde(skip)]
    pub hidden_params: HiddenParams,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ExecutionError {
    pub name: Option<Box<RawValue>>,
    pub value: Option<Box<RawValue>>,
    pub traceback: Option<Box<RawValue>>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CodeExecutionResult {
    #[serde(default)]
    pub stdout: String,
    #[serde(default)]
    pub stderr: String,
    #[serde(default)]
    pub results: Vec<BTreeMap<String, Box<RawValue>>>,
    #[serde(default)]
    pub error: Option<ExecutionError>,
    #[serde(default)]
    pub execution_count: Option<i64>,
    #[serde(default = "default_object")]
    pub object: String,
}

impl Default for CodeExecutionResult {
    fn default() -> Self {
        Self {
            stdout: String::new(),
            stderr: String::new(),
            results: Vec::new(),
            error: None,
            execution_count: None,
            object: CODE_EXECUTION_OBJECT.to_owned(),
        }
    }
}

fn default_object() -> String {
    CODE_EXECUTION_OBJECT.to_owned()
}

#[derive(Debug, Error)]
pub enum SandboxOutputError<Source> {
    #[error("sandbox output exceeded {limit} bytes; aborting to avoid unbounded memory use")]
    Exceeded { limit: usize },
    #[error("the sandbox output is not valid UTF-8")]
    Utf8 {
        #[source]
        source: FromUtf8Error,
    },
    #[error("the sandbox output could not be read")]
    Read {
        #[source]
        source: Source,
    },
}

pub async fn read_capped_lines<Chunks, Source>(
    stream: Chunks,
) -> Result<Vec<String>, SandboxOutputError<Source>>
where
    Chunks: Stream<Item = Result<Vec<u8>, Source>>,
{
    let mut chunks = pin!(stream);
    let mut lines = Vec::new();
    let mut pending = Vec::new();
    let mut total = 0_usize;
    while let Some(item) = chunks.next().await {
        let data = item.map_err(|source| SandboxOutputError::Read { source })?;
        for byte in data {
            if byte == b'\n' {
                total = finish_line(&mut lines, &mut pending, total)?;
            } else {
                pending.push(byte);
            }
        }
        if total.saturating_add(pending.len()) > SANDBOX_MAX_OUTPUT_BYTES {
            return Err(SandboxOutputError::Exceeded {
                limit: SANDBOX_MAX_OUTPUT_BYTES,
            });
        }
    }
    if !pending.is_empty() {
        finish_line(&mut lines, &mut pending, total)?;
    }
    Ok(lines)
}

fn finish_line<Source>(
    lines: &mut Vec<String>,
    pending: &mut Vec<u8>,
    total: usize,
) -> Result<usize, SandboxOutputError<Source>> {
    if pending.last() == Some(&b'\r') {
        pending.pop();
    }
    let text = String::from_utf8(mem::take(pending))
        .map_err(|source| SandboxOutputError::Utf8 { source })?;
    let counted = total
        .checked_add(text.len())
        .ok_or(SandboxOutputError::Exceeded {
            limit: SANDBOX_MAX_OUTPUT_BYTES,
        })?;
    if counted > SANDBOX_MAX_OUTPUT_BYTES {
        return Err(SandboxOutputError::Exceeded {
            limit: SANDBOX_MAX_OUTPUT_BYTES,
        });
    }
    lines.push(text);
    Ok(counted)
}

mod tests;
