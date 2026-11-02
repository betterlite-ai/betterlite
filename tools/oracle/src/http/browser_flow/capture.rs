use std::str;

use secrecy::SecretString;
use url::Url;

use super::{BrowserFlowError, CapturedValue, CookieJar};
use crate::case::browser_flow::form::FormProjection;
use crate::case::browser_flow::{Capture, CaptureSource, MAX_CAPTURE_BYTES};
use crate::http::Response;

// Bundled so `extract` takes the capture itself as its only other argument.
#[derive(Clone, Copy)]
pub(crate) struct StepContext<'context> {
    pub(crate) index: usize,
    pub(crate) response: &'context Response,
    pub(crate) raw_headers: &'context [(String, String)],
    pub(crate) jar: &'context CookieJar,
    pub(crate) form: Option<&'context FormProjection>,
    pub(crate) base: &'context Url,
}

pub(crate) fn extract(
    context: StepContext<'_>,
    capture: &Capture,
) -> Result<CapturedValue, BrowserFlowError> {
    let index = context.index;
    let missing = || BrowserFlowError::MissingCapture {
        index,
        name: capture.name.clone(),
    };
    let text = match &capture.source {
        CaptureSource::Body { pointer } => context
            .response
            .body
            .pointer(pointer)
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
            .ok_or_else(missing)?,
        CaptureSource::Header { name } => {
            header_capture(index, capture, context.raw_headers, name)?
        }
        CaptureSource::Cookie { name } => context
            .jar
            .store
            .get_request_values(context.base)
            .find(|(cookie_name, _)| cookie_name == name)
            .map(|(_, value)| value.to_owned())
            .ok_or_else(missing)?,
        CaptureSource::LocationQuery { name } => location_query(context, capture, name)?,
        CaptureSource::Form { control } => form_control(index, capture, context.form, control)?,
    };
    if text.len() > MAX_CAPTURE_BYTES {
        return Err(BrowserFlowError::CaptureTooLarge {
            index,
            name: capture.name.clone(),
            max: MAX_CAPTURE_BYTES,
        });
    }
    Ok(CapturedValue {
        text: SecretString::from(text),
        secret: capture.secret,
    })
}

// A duplicate header could mean either proxy answer, so it fails the case instead of picking one.
fn header_capture(
    index: usize,
    capture: &Capture,
    raw_headers: &[(String, String)],
    name: &str,
) -> Result<String, BrowserFlowError> {
    let missing = || BrowserFlowError::MissingCapture {
        index,
        name: capture.name.clone(),
    };
    let mut matches = raw_headers
        .iter()
        .filter(|(header_name, _)| header_name.eq_ignore_ascii_case(name));
    let first = matches.next().ok_or_else(missing)?;
    if matches.next().is_some() {
        return Err(BrowserFlowError::AmbiguousCapture {
            index,
            name: capture.name.clone(),
        });
    }
    Ok(first.1.clone())
}

// Location is observed data: its query parameters come from the parsed URL, never a fetch.
fn location_query(
    context: StepContext<'_>,
    capture: &Capture,
    name: &str,
) -> Result<String, BrowserFlowError> {
    let index = context.index;
    let missing = || BrowserFlowError::MissingCapture {
        index,
        name: capture.name.clone(),
    };
    let location = context
        .response
        .headers
        .get("location")
        .ok_or_else(missing)?;
    let resolved = context
        .base
        .join(location)
        .map_err(|source| BrowserFlowError::Location { index, source })?;
    let mut matches = resolved
        .query_pairs()
        .filter(|(key, _)| key == name)
        .map(|(_, value)| value.into_owned());
    let first = matches.next().ok_or_else(missing)?;
    if matches.next().is_some() {
        return Err(BrowserFlowError::AmbiguousCapture {
            index,
            name: capture.name.clone(),
        });
    }
    Ok(first)
}

fn form_control(
    index: usize,
    capture: &Capture,
    form: Option<&FormProjection>,
    control: &str,
) -> Result<String, BrowserFlowError> {
    let missing = || BrowserFlowError::MissingCapture {
        index,
        name: capture.name.clone(),
    };
    let projection = form.ok_or_else(missing)?;
    let mut matches = projection
        .controls
        .iter()
        .filter(|(name, _)| name == control);
    let first = matches.next().ok_or_else(missing)?;
    if matches.next().is_some() {
        return Err(BrowserFlowError::AmbiguousCapture {
            index,
            name: capture.name.clone(),
        });
    }
    first.1.clone().ok_or_else(missing)
}
