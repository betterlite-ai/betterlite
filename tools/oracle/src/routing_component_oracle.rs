use std::env::current_exe;
use std::ffi::OsString;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::Value;
use tokio::runtime::Builder;

use crate::OracleError;
use crate::case::{Ignore, validate_ignores};
use crate::cli::Options;
use crate::compare::{Difference, first_difference, remove_pointer};
use crate::oracle_comparison_contracts::{ExpectedDifference, NumericRule, validate};
use crate::process::sync_litellm;
use crate::router_probes::{ProbeOperation, Reply, Request, Sample, comparison};
use crate::routing_probe_ipc::child::{ProbeCommand, run_bounded};
use crate::routing_probe_ipc::{BUILD_DEADLINE, OPERATION_DEADLINE, ProbeClock, parse_message};

mod tests;

pub(crate) const PROBE_BINARY: &str = "betterlite-router-probe";

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProbeCase {
    kind: String,
    name: String,
    probes: Vec<ProbeStep>,
    #[serde(default)]
    compare: ProbeCompare,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProbeStep {
    clock: ProbeClock,
    samples: Vec<Sample>,
    operation: ProbeOperation,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProbeCompare {
    #[serde(default)]
    ignore: Vec<Ignore>,
    #[serde(default)]
    numeric: Vec<NumericRule>,
    #[serde(default)]
    expected_difference: Vec<ExpectedDifference>,
}

pub(crate) fn validate_probe_case(path: &Path, case: &ProbeCase) -> Result<(), OracleError> {
    let invalid = |message: &str| OracleError::CaseInvalid {
        path: path.to_owned(),
        message: message.to_owned(),
    };
    if case.kind != "probe" {
        return Err(invalid("component probe cases must have kind: probe"));
    }
    if case.name.is_empty() {
        return Err(invalid("name cannot be empty"));
    }
    if case.probes.is_empty() {
        return Err(invalid("probes cannot be empty"));
    }
    validate_ignores(path, &case.compare.ignore)?;
    validate(&case.compare.numeric, &case.compare.expected_difference)
        .map_err(|error| invalid(&error.to_string()))
}

pub(crate) fn run_cases(options: &Options, cases: &[ProbeCase]) -> Result<Vec<bool>, OracleError> {
    if cases.is_empty() {
        return Ok(Vec::new());
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    sync_litellm(&root)?;
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(OracleError::ProbeRuntime)?;
    let commands = Commands::prepare(&root)?;
    runtime.block_on(async {
        build_probe().await?;
        let mut outcomes = Vec::with_capacity(cases.len());
        for case in cases {
            outcomes.push(run_case(options, &commands, case).await?);
        }
        Ok(outcomes)
    })
}

async fn build_probe() -> Result<(), OracleError> {
    let output = run_bounded(&build_command(), b"", BUILD_DEADLINE).await?;
    if !output.status.success() {
        return Err(OracleError::ProbeBuild {
            status: output.status,
            stderr: output.stderr.bytes,
        });
    }
    Ok(())
}

async fn run_case(
    options: &Options,
    commands: &Commands,
    case: &ProbeCase,
) -> Result<bool, OracleError> {
    let mut different = false;
    for (index, step) in case.probes.iter().enumerate() {
        different |= run_step(options, commands, case, index, step).await?;
    }
    Ok(different)
}

async fn run_step(
    options: &Options,
    commands: &Commands,
    case: &ProbeCase,
    index: usize,
    step: &ProbeStep,
) -> Result<bool, OracleError> {
    let request = Request {
        clock: step.clock,
        samples: step.samples.clone(),
        operation: step.operation.clone(),
    };
    let bytes = serde_json::to_vec(&request).map_err(OracleError::Json)?;
    let (first, second) = if options.self_check {
        (
            run_probe(&commands.python, &bytes).await?,
            run_probe(&commands.python, &bytes).await?,
        )
    } else {
        (
            run_probe(&commands.native, &bytes).await?,
            run_probe(&commands.python, &bytes).await?,
        )
    };
    match compare_probe_replies(&first, &second, &case.compare, options.self_check)? {
        None => {
            writeln!(io::stdout().lock(), "PASS {}[{index}]", case.name)
                .map_err(OracleError::Output)?;
            Ok(false)
        }
        Some(difference) => {
            writeln!(
                io::stderr().lock(),
                "FAIL {}[{index}]: {}",
                case.name,
                difference.pointer
            )
            .map_err(OracleError::Output)?;
            Ok(true)
        }
    }
}

async fn run_probe(command: &ProbeCommand, bytes: &[u8]) -> Result<Reply, OracleError> {
    let output = run_bounded(command, bytes, OPERATION_DEADLINE).await?;
    if !output.status.success() {
        return Err(OracleError::ProbeExited {
            status: output.status,
            stderr: output.stderr.bytes,
        });
    }
    if output.stdout.truncated {
        return Err(OracleError::ProbeTooLarge);
    }
    parse_message(&output.stdout.bytes)
}

fn compare_probe_replies(
    first: &Reply,
    second: &Reply,
    compare: &ProbeCompare,
    self_check: bool,
) -> Result<Option<Difference>, OracleError> {
    if compare.numeric.is_empty() && compare.expected_difference.is_empty() {
        return compare_replies(first, second, &compare.ignore);
    }
    let (first_value, second_value) = reply_values(first, second)?;
    Ok(comparison::compare(&comparison::Documents {
        reference: &second_value,
        rust: &first_value,
        numeric: &compare.numeric,
        expected_difference: &compare.expected_difference,
        ignore: &compare.ignore,
        self_check,
        message: "probe results differ",
    }))
}

fn reply_values(first: &Reply, second: &Reply) -> Result<(Value, Value), OracleError> {
    let first_value = serde_json::to_value(first).map_err(OracleError::Json)?;
    let second_value = serde_json::to_value(second).map_err(OracleError::Json)?;
    Ok((first_value, second_value))
}

fn compare_replies(
    first: &Reply,
    second: &Reply,
    ignores: &[Ignore],
) -> Result<Option<Difference>, OracleError> {
    let (mut first_value, mut second_value) = reply_values(first, second)?;
    for ignored in ignores {
        remove_pointer(&mut first_value, &ignored.pointer);
        remove_pointer(&mut second_value, &ignored.pointer);
    }
    Ok(
        first_difference(&first_value, &second_value, "").map(|pointer| Difference {
            message: "probe results differ".to_owned(),
            pointer,
        }),
    )
}

struct Commands {
    native: ProbeCommand,
    python: ProbeCommand,
}

impl Commands {
    fn prepare(root: &Path) -> Result<Self, OracleError> {
        let executable = current_exe().map_err(OracleError::ProbeExecutable)?;
        let directory = executable
            .parent()
            .ok_or_else(|| OracleError::ProbeBinary {
                directory: executable.clone(),
            })?;
        let native = ProbeCommand {
            program: directory.join(PROBE_BINARY),
            args: Vec::new(),
            cwd: root.to_owned(),
        };
        Ok(Self {
            native,
            python: python_command(root),
        })
    }
}

fn python_command(root: &Path) -> ProbeCommand {
    ProbeCommand {
        program: root.join("tools/oracle/litellm/.venv/bin/python"),
        args: vec![OsString::from("-m"), OsString::from("probes")],
        cwd: root.join("tools/oracle/litellm"),
    }
}

fn build_command() -> ProbeCommand {
    let arguments = [
        "build",
        "--locked",
        "--quiet",
        "-p",
        "betterlite-oracle",
        "--bin",
        PROBE_BINARY,
    ];
    ProbeCommand {
        program: PathBuf::from("cargo"),
        args: arguments.map(OsString::from).to_vec(),
        cwd: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."),
    }
}
