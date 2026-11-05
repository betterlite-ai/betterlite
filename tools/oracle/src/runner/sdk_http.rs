use std::io::{self, Write as _};
use std::path::Path;
use std::sync::atomic::AtomicBool;

use serde_json::Value;

use crate::OracleError;
use crate::artifacts::{self, SdkArtifactInput};
use crate::cli::Options;
use crate::compare::{self, Difference};
use crate::paths::artifact_root;
use crate::routing_mock_scripts::MockScript;
use crate::sdk_http::capture::{Capture, Implementation, Run, capture, observation};
use crate::sdk_http::native::NativeSdk;
use crate::sdk_http::python::{ProcessEnvironment, PythonSdk};
use crate::sdk_http::{OracleLimits, SdkHttpCase, Side, sides};

pub(crate) fn run_cases(
    options: &Options,
    cases: &[SdkHttpCase],
) -> Result<Vec<bool>, OracleError> {
    let cancelled = AtomicBool::new(false);
    cases
        .iter()
        .map(|case| run_case(options, case, &cancelled))
        .collect()
}

fn run_case(
    options: &Options,
    case: &SdkHttpCase,
    cancelled: &AtomicBool,
) -> Result<bool, OracleError> {
    let limits = OracleLimits::default();
    let python = PythonSdk {
        root: Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."),
        inherited: <ProcessEnvironment as clap::Parser>::try_parse_from([env!("CARGO_PKG_NAME")])?,
        case_env: case.env.clone(),
        limits: limits.clone(),
    };
    let [first, second] = sides(options.self_check);
    let mut captures = Vec::new();
    for side in [first, second] {
        let implementation: &dyn Implementation = match side {
            Side::Python => &python,
            Side::Native => &NativeSdk,
        };
        let run = Run {
            call: &case.call,
            script: MockScript::try_from(case.upstream.as_slice())?,
            limits: &limits,
            cancelled,
        };
        captures.push(capture(implementation, run)?);
    }
    let [left, right] = &*captures else {
        return Ok(false);
    };
    let Err(difference) = judge(left, right, case) else {
        writeln!(io::stdout().lock(), "PASS {}", case.name).map_err(OracleError::Output)?;
        return Ok(false);
    };
    report_failure(options, case, [left, right], &difference)?;
    Ok(true)
}

fn report_failure(
    options: &Options,
    case: &SdkHttpCase,
    captures: [&Capture; 2],
    difference: &Difference,
) -> Result<(), OracleError> {
    let labels = [
        "litellm",
        if options.self_check {
            "litellm-second"
        } else {
            "betterlite"
        },
    ];
    let directory = artifacts::write_sdk(&SdkArtifactInput {
        root: &artifact_root()?,
        name: &case.name,
        labels,
        captures,
        difference,
    })?;
    writeln!(
        io::stderr().lock(),
        "FAIL {}: {} ({})",
        case.name,
        difference.pointer,
        directory.display()
    )
    .map_err(OracleError::Output)
}

pub(crate) fn judge(left: &Capture, right: &Capture, case: &SdkHttpCase) -> Result<(), Difference> {
    let headers = &case.compare.upstream_headers;
    let left_value = observe("left", left, headers)?;
    let right_value = observe("right", right, headers)?;
    compare::compare_sdk(&left_value, &right_value, &case.compare.ignore)
}

fn observe(label: &str, side: &Capture, headers: &[String]) -> Result<Value, Difference> {
    let result = side.execution.result.as_ref().map_err(|error| Difference {
        pointer: format!("/execution/{label}"),
        message: error.to_string(),
    })?;
    if let Some(failure) = &side.mock_failure {
        return Err(Difference {
            pointer: "/upstream".to_owned(),
            message: failure.to_string(),
        });
    }
    observation(result, &side.requests, headers).map_err(|source| Difference {
        pointer: format!("/execution/{label}"),
        message: source.to_string(),
    })
}
