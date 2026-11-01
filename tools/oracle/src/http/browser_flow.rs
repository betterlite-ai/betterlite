use std::collections::BTreeMap;
use std::fmt::{self, Debug, Formatter};
use std::path::PathBuf;

use reqwest::blocking::Client;
use thiserror::Error;

use super::Response;
use crate::OracleError;
use crate::case::Request;
use crate::case::browser_flow::{FormCaptureError, Step, parse_form_projection};

mod capture;
mod interpolate;
pub(crate) mod tests;

#[derive(Debug, Error)]
pub(crate) enum BrowserFlowError {
    #[error("step {index} capture \"{name}\" did not match any value")]
    MissingCapture { index: usize, name: String },
    #[error("step {index} capture \"{name}\" matched more than one value")]
    AmbiguousCapture { index: usize, name: String },
    #[error("step {index} capture \"{name}\" exceeds {max} bytes")]
    CaptureTooLarge {
        index: usize,
        name: String,
        max: usize,
    },
    #[error("step {index}: {reason}")]
    Interpolate { index: usize, reason: String },
    #[error("step {index} form: {source}")]
    Form {
        index: usize,
        #[source]
        source: FormCaptureError,
    },
}

impl From<BrowserFlowError> for OracleError {
    fn from(error: BrowserFlowError) -> Self {
        Self::CaseInvalid {
            path: PathBuf::from("<browser-flow>"),
            message: error.to_string(),
        }
    }
}

pub(crate) struct CapturedValue {
    text: String,
    secret: bool,
}

impl Debug for CapturedValue {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        if self.secret {
            f.write_str("CapturedValue(redacted)")
        } else {
            f.debug_struct("CapturedValue")
                .field("text", &self.text)
                .finish()
        }
    }
}

#[derive(Debug, Default)]
pub(crate) struct CookieJar {
    cookies: BTreeMap<String, String>,
}

impl CookieJar {
    fn update_from_response(&mut self, response: &Response) {
        let Some(set_cookie) = response.headers.get("set-cookie") else {
            return;
        };
        let pair = set_cookie.split(';').next().unwrap_or_default();
        if let Some((name, value)) = pair.split_once('=') {
            self.cookies
                .insert(name.trim().to_owned(), value.trim().to_owned());
        }
    }

    fn header_value(&self) -> Option<String> {
        if self.cookies.is_empty() {
            return None;
        }
        Some(
            self.cookies
                .iter()
                .map(|(name, value)| format!("{name}={value}"))
                .collect::<Vec<_>>()
                .join("; "),
        )
    }
}

struct StepState<'state> {
    jar: &'state mut CookieJar,
    captures: &'state mut BTreeMap<String, CapturedValue>,
}

// The jar and capture map are local to this call, so the two proxies never share either.
pub(crate) fn run_case_steps(
    client: &Client,
    base: &str,
    steps: &[Step],
) -> Result<Vec<Response>, OracleError> {
    let mut jar = CookieJar::default();
    let mut captures = BTreeMap::new();
    let mut responses = Vec::with_capacity(steps.len());
    for (index, step) in steps.iter().enumerate() {
        let mut state = StepState {
            jar: &mut jar,
            captures: &mut captures,
        };
        responses.push(run_step(client, base, index, step, &mut state)?);
    }
    Ok(responses)
}

fn run_step(
    client: &Client,
    base: &str,
    index: usize,
    step: &Step,
    state: &mut StepState<'_>,
) -> Result<Response, OracleError> {
    let path = interpolate::resolve_text(index, &step.request.path, state.captures)?;
    let mut headers = step
        .request
        .headers
        .iter()
        .map(|(name, value)| {
            Ok((
                name.clone(),
                interpolate::resolve_text(index, value, state.captures)?,
            ))
        })
        .collect::<Result<BTreeMap<_, _>, BrowserFlowError>>()?;
    if let Some(cookie_header) = state.jar.header_value() {
        headers.insert("cookie".to_owned(), cookie_header);
    }
    let body = interpolate::resolve_body(index, step, state.captures)?;
    let request = Request {
        method: step.request.method.clone(),
        path,
        headers,
        body: serde_json::Value::Null,
        body_kind: step.request.body_kind,
    };
    let response = super::send_with_body(client, base, &request, body)?;
    state.jar.update_from_response(&response);
    apply_captures(index, step, &response, base, state)?;
    Ok(response)
}

fn apply_captures(
    index: usize,
    step: &Step,
    response: &Response,
    base: &str,
    state: &mut StepState<'_>,
) -> Result<(), OracleError> {
    let form = if step.form {
        let text = response.body.as_str().ok_or(BrowserFlowError::Form {
            index,
            source: FormCaptureError::FormCount { count: 0 },
        })?;
        Some(
            parse_form_projection(text)
                .map_err(|source| BrowserFlowError::Form { index, source })?,
        )
    } else {
        None
    };
    let context = capture::StepContext {
        index,
        response,
        jar: state.jar,
        form: form.as_ref(),
        base,
    };
    for capture in &step.captures {
        let value = capture::extract(context, capture)?;
        state.captures.insert(capture.name.clone(), value);
    }
    Ok(())
}
