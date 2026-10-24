mod bedrock_request_policy;
mod catalog_generalizations;
mod dot_paths;
mod provider_base_helpers;
mod provider_message_primitives;
mod proxy_policy;
mod url_path;

use std::collections::BTreeMap;
use std::io::{self, Read, Write as _};
use std::num::NonZeroUsize;
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::OracleError;
use crate::case::HelperCase;
use crate::compare;
use crate::process::sync_litellm;

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) enum HelperCall {
    #[serde(rename = "bedrock_request_policy")]
    BedrockRequestPolicy(bedrock_request_policy::BedrockRequestPolicyInput),
    #[serde(rename = "catalog_generalizations")]
    CatalogGeneralizations(catalog_generalizations::CatalogGeneralizationsInput),
    #[serde(rename = "dot_paths")]
    DotPaths(dot_paths::DotPathsInput),
    #[serde(rename = "provider_base_helpers")]
    ProviderBaseHelpers(provider_base_helpers::BaseCall),
    #[serde(rename = "provider_message_primitives")]
    ProviderMessagePrimitives(provider_message_primitives::PrimitivesInput),
    #[serde(rename = "proxy_policy")]
    ProxyPolicy(proxy_policy::ProxyPolicyInput),
    #[serde(rename = "url_path")]
    UrlPath(url_path::UrlPathInput),
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub(crate) struct HelperOutput {
    pub(crate) ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) value: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) error: Option<HelperError>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub(crate) struct HelperError {
    pub(crate) class: String,
    pub(crate) status: Option<u16>,
    #[serde(rename = "type")]
    pub(crate) error_type: Option<String>,
    pub(crate) param: Option<String>,
    pub(crate) code: Option<String>,
    pub(crate) message: String,
}

pub(crate) enum HelperRun {
    Match,
    Difference,
}

pub(crate) fn run_case(case: &HelperCase, self_check: bool) -> Result<HelperRun, OracleError> {
    let call_json = serde_json::to_value(&case.call).map_err(OracleError::Json)?;
    let Some((operation, input_value)) = call_json.as_object().and_then(|call| call.iter().next())
    else {
        return Err(OracleError::HelperInput {
            message: "helper call is not a tagged object".to_owned(),
        });
    };
    let python_input = serde_json::json!({"operation": operation, "input": input_value});
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    sync_litellm(&root)?;
    let left = if self_check {
        run_python(&root, &python_input, &case.env)?
    } else {
        invoke(&case.call)
    };
    let right = run_python(&root, &python_input, &case.env)?;
    if let Some(difference) = compare::compare_helper(&left, &right, &case.compare.ignore) {
        let pointer = difference.pointer;
        writeln!(io::stderr().lock(), "FAIL {}: {pointer}", case.name)
            .map_err(OracleError::Output)?;
        return Ok(HelperRun::Difference);
    }
    writeln!(io::stdout().lock(), "PASS {}", case.name).map_err(OracleError::Output)?;
    Ok(HelperRun::Match)
}

fn invoke(call: &HelperCall) -> HelperOutput {
    let result = match call {
        HelperCall::BedrockRequestPolicy(input) => bedrock_request_policy::invoke(input),
        HelperCall::CatalogGeneralizations(input) => return catalog_generalizations::invoke(input),
        HelperCall::DotPaths(input) => dot_paths::invoke(input),
        HelperCall::ProviderBaseHelpers(input) => provider_base_helpers::invoke(input),
        HelperCall::ProviderMessagePrimitives(input) => provider_message_primitives::invoke(input),
        HelperCall::ProxyPolicy(input) => proxy_policy::invoke(input),
        HelperCall::UrlPath(input) => url_path::invoke(input),
    };
    match result {
        Ok(value) => HelperOutput {
            ok: true,
            value: Some(value),
            error: None,
        },
        Err(error) => HelperOutput {
            ok: false,
            value: None,
            error: Some(*error),
        },
    }
}

fn run_python(
    root: &Path,
    input: &serde_json::Value,
    case_env: &BTreeMap<String, String>,
) -> Result<HelperOutput, OracleError> {
    let mut command = Command::new("setpriv");
    command
        .args(["--pdeathsig", "KILL", "--"])
        .arg(root.join("tools/oracle/litellm/.venv/bin/python"))
        .arg(root.join("tools/oracle/helpers/dispatch.py"))
        .env_clear();
    configure_environment(&mut command, case_env);
    let bytes = serde_json::to_vec(input).map_err(OracleError::Json)?;
    let stdout_output = run_bounded(&mut command, bytes, Duration::from_secs(15))?;
    serde_json::from_slice(&stdout_output).map_err(OracleError::HelperJson)
}

fn run_bounded(
    command: &mut Command,
    input: Vec<u8>,
    timeout: Duration,
) -> Result<Vec<u8>, OracleError> {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(OracleError::HelperSpawn)?;
    let pumped = pump_helper_pipes(&mut child, input);
    let status = wait_for_helper_exit(&mut child, timeout)?;
    let pipes = pumped?;
    let (stdout_output, stdout_excess) = pipes
        .output
        .join()
        .map_err(|_| OracleError::HelperJoin)?
        .map_err(OracleError::HelperIo)?;
    let (stderr_output, _stderr_excess) = pipes
        .diagnostic
        .join()
        .map_err(|_| OracleError::HelperJoin)?
        .map_err(OracleError::HelperIo)?;
    if stdout_excess {
        return Err(OracleError::HelperOutputTooLarge);
    }
    if !status.success() {
        return Err(OracleError::HelperExited {
            status,
            diagnostic: String::from_utf8_lossy(&stderr_output).into_owned(),
        });
    }
    pipes
        .input
        .join()
        .map_err(|_| OracleError::HelperJoin)?
        .map_err(OracleError::HelperIo)?;
    Ok(stdout_output)
}

struct HelperPipes {
    input: JoinHandle<io::Result<()>>,
    output: JoinHandle<io::Result<(Vec<u8>, bool)>>,
    diagnostic: JoinHandle<io::Result<(Vec<u8>, bool)>>,
}

fn pump_helper_pipes(child: &mut Child, input: Vec<u8>) -> Result<HelperPipes, OracleError> {
    let mut stdin_pipe = child.stdin.take().ok_or(OracleError::HelperStdin)?;
    let stdout_pipe = child.stdout.take().ok_or(OracleError::HelperStdout)?;
    let stderr_pipe = child.stderr.take().ok_or(OracleError::HelperStderr)?;
    Ok(HelperPipes {
        input: thread::spawn(move || stdin_pipe.write_all(&input)),
        output: thread::spawn(move || read_limited(stdout_pipe)),
        diagnostic: thread::spawn(move || read_limited(stderr_pipe)),
    })
}

fn wait_for_helper_exit(child: &mut Child, timeout: Duration) -> Result<ExitStatus, OracleError> {
    let started = Instant::now();
    loop {
        if let Some(exit_status) = child.try_wait().map_err(OracleError::HelperIo)? {
            return Ok(exit_status);
        }
        if started.elapsed() >= timeout {
            child.kill().map_err(OracleError::HelperIo)?;
            child.wait().map_err(OracleError::HelperIo)?;
            return Err(OracleError::HelperTimeout);
        }
        thread::park_timeout(Duration::from_millis(10));
    }
}

fn configure_environment(command: &mut Command, case_env: &BTreeMap<String, String>) {
    for name in ["PATH", "HOME", "UV_CACHE_DIR", "UV_PYTHON_CACHE_DIR"] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    for (name, value) in case_env {
        command.env(name, value);
    }
    command.env("LITELLM_LOCAL_MODEL_COST_MAP", "True");
}

fn read_limited<R>(mut reader: R) -> io::Result<(Vec<u8>, bool)>
where
    R: Read,
{
    const MAX_OUTPUT: usize = 1024 * 1024;
    let mut output = Vec::new();
    let mut buffer = [0_u8; 8192];
    let mut excess = false;
    loop {
        let Some(nonzero_count) = NonZeroUsize::new(reader.read(&mut buffer)?) else {
            return Ok((output, excess));
        };
        let count = nonzero_count.get();
        let remaining = MAX_OUTPUT.saturating_sub(output.len());
        let bytes = buffer
            .get(..count.min(remaining))
            .ok_or_else(|| io::Error::other("reader returned an invalid byte count"))?;
        output.extend(bytes.iter().copied());
        excess |= count > remaining;
    }
}

mod tests;
