use std::collections::BTreeMap;
use std::fmt::{self, Debug, Formatter};
use std::path::PathBuf;
use std::str::Utf8Error;
use std::time::Instant;

use cookie_store::{CookieError, CookieStore};
use reqwest::blocking::Client;
use secrecy::{ExposeSecret as _, SecretString};
use thiserror::Error;
use url::Url;

use super::{REQUEST_TIMEOUT, Response, send_with_raw_headers};
use crate::OracleError;
use crate::case::Request;
use crate::case::browser_flow::form::{FormCaptureError, parse_form_projection};
use crate::case::browser_flow::{PlaceholderError, Step};
use crate::multipart_oracle::error::MultipartError;

mod capture;
mod interpolate;
pub(crate) mod tests;

#[derive(Debug, Error)]
pub(crate) enum BrowserFlowError {
    #[error("the gateway base url is invalid: {0}")]
    BaseUrl(#[source] url::ParseError),
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
    #[error("step {index} location header is not a valid url: {source}")]
    Location {
        index: usize,
        #[source]
        source: url::ParseError,
    },
    #[error("step {index} declares its own cookie header while the jar is in use")]
    CookieConflict { index: usize },
    #[error("step {index} set-cookie header is invalid: {source}")]
    CookieInsert {
        index: usize,
        #[source]
        source: CookieError,
    },
    #[error("step {index}: {source}")]
    Placeholder {
        index: usize,
        #[source]
        source: PlaceholderError,
    },
    #[error("step {index}: unterminated capture placeholder")]
    Interpolate { index: usize },
    #[error("step {index} body: {source}")]
    Multipart {
        index: usize,
        #[source]
        source: MultipartError,
    },
    #[error("step {index} fixture body is not valid UTF-8: {source}")]
    BodyUtf8 {
        index: usize,
        #[source]
        source: Utf8Error,
    },
    #[error("step {index} bytes body did not decode to raw bytes")]
    BytesBody { index: usize },
    #[error("step {index} form: {source}")]
    Form {
        index: usize,
        #[source]
        source: FormCaptureError,
    },
    #[error("the transcript ran past its deadline at step {index}")]
    DeadlineExceeded { index: usize },
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
    text: SecretString,
    secret: bool,
}

impl Debug for CapturedValue {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        if self.secret {
            f.write_str("CapturedValue(redacted)")
        } else {
            f.debug_struct("CapturedValue")
                .field("text", &self.text.expose_secret())
                .finish()
        }
    }
}

// cookie_store::CookieStore understands Max-Age and Expires, so a logout step's clearing cookie sticks.
pub(crate) struct CookieJar {
    store: CookieStore,
}

impl Default for CookieJar {
    fn default() -> Self {
        Self {
            store: CookieStore::new(),
        }
    }
}

impl Debug for CookieJar {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str("CookieJar(redacted)")
    }
}

impl CookieJar {
    fn update_from_response(
        &mut self,
        index: usize,
        base: &Url,
        raw_headers: &[(String, String)],
    ) -> Result<(), BrowserFlowError> {
        for (name, value) in raw_headers {
            if !name.eq_ignore_ascii_case("set-cookie") {
                continue;
            }
            match self.store.parse(value, base) {
                Ok(_) | Err(CookieError::Expired) => {}
                Err(source) => return Err(BrowserFlowError::CookieInsert { index, source }),
            }
        }
        Ok(())
    }

    fn header_value(&self, base: &Url) -> Option<String> {
        let mut pairs = self.store.get_request_values(base).peekable();
        pairs.peek()?;
        Some(
            pairs
                .map(|(name, value)| format!("{name}={value}"))
                .collect::<Vec<_>>()
                .join("; "),
        )
    }
}

struct StepState<'state> {
    base_url: &'state Url,
    jar: &'state mut CookieJar,
    captures: &'state mut BTreeMap<String, CapturedValue>,
}

// The jar and capture map are local to this call, so the two proxies never share either.
pub(crate) fn run_case_steps(
    client: &Client,
    base: &str,
    steps: &[Step],
) -> Result<Vec<Response>, OracleError> {
    let base_url = Url::parse(base).map_err(BrowserFlowError::BaseUrl)?;
    let deadline = Instant::now()
        .checked_add(REQUEST_TIMEOUT)
        .ok_or(BrowserFlowError::DeadlineExceeded { index: 0 })?;
    let mut jar = CookieJar::default();
    let mut captures = BTreeMap::new();
    let mut responses = Vec::with_capacity(steps.len());
    for (index, step) in steps.iter().enumerate() {
        if Instant::now() >= deadline {
            return Err(OracleError::from(BrowserFlowError::DeadlineExceeded {
                index,
            }));
        }
        let mut state = StepState {
            base_url: &base_url,
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
    if let Some(cookie_header) = state.jar.header_value(state.base_url) {
        if headers
            .keys()
            .any(|name| name.eq_ignore_ascii_case("cookie"))
        {
            return Err(OracleError::from(BrowserFlowError::CookieConflict {
                index,
            }));
        }
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
    let (response, raw_headers) = send_with_raw_headers(client, base, &request, body)?;
    state
        .jar
        .update_from_response(index, state.base_url, &raw_headers)?;
    apply_captures(index, step, &response, &raw_headers, state)?;
    Ok(response)
}

fn apply_captures(
    index: usize,
    step: &Step,
    response: &Response,
    raw_headers: &[(String, String)],
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
        raw_headers,
        jar: state.jar,
        form: form.as_ref(),
        base: state.base_url,
    };
    for capture in &step.captures {
        let value = capture::extract(context, capture)?;
        state.captures.insert(capture.name.clone(), value);
    }
    Ok(())
}
