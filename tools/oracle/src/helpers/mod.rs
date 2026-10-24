mod url_path;

use std::collections::BTreeMap;
use std::io::{self, Read, Write as _};
use std::num::NonZeroUsize;
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::OracleError;
use crate::case::HelperCase;
use crate::compare;

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) enum HelperCall {
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
    let object = call_json
        .as_object()
        .ok_or_else(|| OracleError::HelperInput {
            message: "helper call is not an object".to_owned(),
        })?;
    let (operation, input_value) =
        object
            .iter()
            .next()
            .ok_or_else(|| OracleError::HelperInput {
                message: "helper call has no operation".to_owned(),
            })?;
    let python_input = serde_json::json!({"operation": operation, "input": input_value});
    let rust = invoke(&case.call);
    let first = run_python(&python_input, &case.env)?;
    let python_second = if self_check {
        Some(run_python(&python_input, &case.env)?)
    } else {
        None
    };
    let (left, right) = if let Some(second_output) = python_second {
        (first, second_output)
    } else {
        (rust, first)
    };
    if let Some(difference) = compare::compare_helper(&left, &right, &case.compare.ignore) {
        writeln!(
            io::stderr().lock(),
            "FAIL {}: {}",
            case.name,
            difference.pointer
        )
        .map_err(OracleError::Output)?;
        return Ok(HelperRun::Difference);
    }
    writeln!(io::stdout().lock(), "PASS {}", case.name).map_err(OracleError::Output)?;
    Ok(HelperRun::Match)
}

fn invoke(call: &HelperCall) -> HelperOutput {
    match call {
        HelperCall::UrlPath(input) => match url_path::invoke(input) {
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
        },
    }
}

fn run_python(
    input: &serde_json::Value,
    case_env: &BTreeMap<String, String>,
) -> Result<HelperOutput, OracleError> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let project = root.join("tools/oracle/litellm");
    let dispatch = root.join("tools/oracle/helpers/dispatch.py");
    let mut command = Command::new("setpriv");
    command
        .args([
            "--pdeathsig",
            "KILL",
            "--",
            "uv",
            "run",
            "--locked",
            "--no-dev",
            "--project",
        ])
        .arg(&project)
        .args(["python"])
        .arg(&dispatch)
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    configure_environment(&mut command, case_env);
    let mut child = command.spawn().map_err(OracleError::HelperSpawn)?;
    let bytes = serde_json::to_vec(input).map_err(OracleError::Json)?;
    let mut stdin = child.stdin.take().ok_or(OracleError::HelperStdin)?;
    stdin.write_all(&bytes).map_err(OracleError::HelperIo)?;
    drop(stdin);
    let stdout = child.stdout.take().ok_or(OracleError::HelperStdout)?;
    let stderr = child.stderr.take().ok_or(OracleError::HelperStderr)?;
    let output_reader = thread::spawn(move || read_limited(stdout));
    let error_reader = thread::spawn(move || read_limited(stderr));
    let deadline = Instant::now()
        .checked_add(Duration::from_secs(10))
        .ok_or(OracleError::HelperTimeout)?;
    let status = loop {
        if let Some(status) = child.try_wait().map_err(OracleError::HelperIo)? {
            break status;
        }
        if Instant::now().checked_duration_since(deadline).is_some() {
            let _killed = child.kill();
            let _reaped = child.wait();
            return Err(OracleError::HelperTimeout);
        }
        thread::park_timeout(Duration::from_millis(10));
    };
    let stdout = output_reader
        .join()
        .map_err(|_| OracleError::HelperJoin)?
        .map_err(OracleError::HelperIo)?;
    let stderr = error_reader
        .join()
        .map_err(|_| OracleError::HelperJoin)?
        .map_err(OracleError::HelperIo)?;
    if stdout.1 {
        return Err(OracleError::HelperOutputTooLarge);
    }
    if !status.success() {
        let diagnostic = String::from_utf8_lossy(&stderr.0);
        return Err(OracleError::HelperExited {
            status,
            diagnostic: diagnostic.into_owned(),
        });
    }
    serde_json::from_slice(&stdout.0).map_err(OracleError::HelperJson)
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
        let read_count = NonZeroUsize::new(reader.read(&mut buffer)?);
        let Some(nonzero_count) = read_count else {
            return Ok((output, excess));
        };
        let count = nonzero_count.get();
        let remaining = MAX_OUTPUT.saturating_sub(output.len());
        let bytes = buffer
            .get(..count.min(remaining))
            .ok_or_else(|| io::Error::other("reader returned an invalid byte count"))?;
        output.extend(bytes.iter().copied());
        if count > remaining {
            excess = true;
        }
    }
}

mod tests;
