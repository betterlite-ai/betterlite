use std::collections::BTreeMap;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use serde_json::Value;

use super::error::SdkHttpError;
use super::native;
use super::python::PythonSdk;
use super::result::SdkResult;
use super::{OracleLimits, SdkHttpCall, Side};
use crate::OracleError;
use crate::compare::multipart::masked_boundary;
use crate::mock::{MockProvider, ProviderRequest};
use crate::routing_mock_scripts::MockScript;

pub(crate) struct Execution {
    pub(crate) result: Result<SdkResult, SdkHttpError>,
    pub(crate) log: Vec<u8>,
}

pub(crate) enum Sdk<'sdk> {
    Python(&'sdk PythonSdk),
    Native,
}

impl<'sdk> Sdk<'sdk> {
    pub(crate) const fn for_side(side: Side, python: &'sdk PythonSdk) -> Self {
        match side {
            Side::Python => Self::Python(python),
            Side::Native => Self::Native,
        }
    }

    fn execute(&self, call: &SdkHttpCall, api_base: &str, stop: &Stop<'_>) -> Execution {
        match self {
            Self::Python(python) => python.execute(call, api_base, stop),
            Self::Native => native::execute(call, api_base, stop),
        }
    }
}

pub(crate) struct Stop<'flag> {
    pub(crate) deadline: Instant,
    pub(crate) cancelled: &'flag AtomicBool,
}

impl Stop<'_> {
    pub(crate) fn check(&self) -> Result<(), SdkHttpError> {
        if self.cancelled.load(Ordering::Acquire) {
            return Err(SdkHttpError::Cancelled);
        }
        if Instant::now() >= self.deadline {
            return Err(SdkHttpError::Deadline);
        }
        Ok(())
    }
}

pub(crate) struct Capture {
    pub(crate) execution: Execution,
    pub(crate) requests: Vec<ProviderRequest>,
    pub(crate) mock_failure: Option<io::Error>,
}

pub(crate) struct Run<'run> {
    pub(crate) call: &'run SdkHttpCall,
    pub(crate) script: MockScript,
    pub(crate) limits: &'run OracleLimits,
    pub(crate) cancelled: &'run AtomicBool,
}

// every implementation gets its own mock, so one side's traffic can never fill the other's answers
pub(crate) fn capture(sdk: &Sdk<'_>, run: Run<'_>) -> Result<Capture, OracleError> {
    let mock = MockProvider::start(run.script)?;
    let api_base = format!("{}/v1", mock.base_url);
    let deadline = Instant::now()
        .checked_add(run.limits.max_execution)
        .ok_or(SdkHttpError::Deadline)?;
    let stop = Stop {
        deadline,
        cancelled: run.cancelled,
    };
    let mut execution = sdk.execute(run.call, &api_base, &stop);
    let (calls, mock_failure) = mock.finish_recording()?;
    let requests: Vec<ProviderRequest> = calls.into_iter().map(|call| call.request).collect();
    if let Err(error) = check_captures(&requests, run.limits)
        && execution.result.is_ok()
    {
        execution.result = Err(error);
    }
    Ok(Capture {
        execution,
        requests,
        mock_failure,
    })
}

pub(crate) fn check_captures(
    requests: &[ProviderRequest],
    limits: &OracleLimits,
) -> Result<(), SdkHttpError> {
    if requests.len() > limits.max_requests {
        return Err(SdkHttpError::TooManyRequests {
            count: requests.len(),
            limit: limits.max_requests,
        });
    }
    for (index, request) in requests.iter().enumerate() {
        let bytes = serde_json::to_vec(&request.body)
            .map_err(SdkHttpError::Serialize)?
            .len();
        if bytes > limits.max_body_bytes {
            return Err(SdkHttpError::BodyTooLarge {
                index,
                bytes,
                limit: limits.max_body_bytes,
            });
        }
    }
    Ok(())
}

// the result keeps its own keys and gains the provider traffic, so ignore pointers address both
pub(crate) fn observation(
    result: &SdkResult,
    requests: &[ProviderRequest],
    upstream_headers: &[String],
) -> Result<Value, serde_json::Error> {
    let mut observed = serde_json::to_value(result)?;
    let mut upstream = Vec::with_capacity(requests.len());
    for request in requests {
        let headers: BTreeMap<String, Option<String>> = upstream_headers
            .iter()
            .map(|header| {
                let name = header.to_ascii_lowercase();
                let value = request
                    .headers
                    .get(&name)
                    .map(|value| masked_boundary(value));
                (name, value)
            })
            .collect();
        let mut entry = serde_json::Map::new();
        entry.insert("method".to_owned(), Value::String(request.method.clone()));
        entry.insert("path".to_owned(), Value::String(request.path.clone()));
        entry.insert("headers".to_owned(), serde_json::to_value(&headers)?);
        entry.insert("body".to_owned(), request.body.clone());
        upstream.push(Value::Object(entry));
    }
    if let Value::Object(object) = &mut observed {
        object.insert("upstream".to_owned(), Value::Array(upstream));
    }
    Ok(observed)
}
