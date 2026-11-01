use std::str;

use url::Url;

use super::{BrowserFlowError, CapturedValue, CookieJar};
use crate::case::browser_flow::{Capture, CaptureSource, FormProjection, MAX_CAPTURE_BYTES};
use crate::http::Response;

// Bundled so `extract` takes the capture itself as its only other argument.
#[derive(Clone, Copy)]
pub(crate) struct StepContext<'context> {
    pub(crate) index: usize,
    pub(crate) response: &'context Response,
    pub(crate) jar: &'context CookieJar,
    pub(crate) form: Option<&'context FormProjection>,
    pub(crate) base: &'context str,
}

pub(crate) fn extract(
    context: StepContext<'_>,
    capture: &Capture,
) -> Result<CapturedValue, BrowserFlowError> {
    let index = context.index;
    let text = match &capture.source {
        CaptureSource::Body { pointer } => context
            .response
            .body
            .pointer(pointer)
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| BrowserFlowError::MissingCapture {
                index,
                name: capture.name.clone(),
            })?,
        CaptureSource::Header { name } => context
            .response
            .headers
            .get(&name.to_ascii_lowercase())
            .cloned()
            .ok_or_else(|| BrowserFlowError::MissingCapture {
                index,
                name: capture.name.clone(),
            })?,
        CaptureSource::Cookie { name } => {
            context.jar.cookies.get(name).cloned().ok_or_else(|| {
                BrowserFlowError::MissingCapture {
                    index,
                    name: capture.name.clone(),
                }
            })?
        }
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
        text,
        secret: capture.secret,
    })
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
    let base_url = Url::parse(context.base).map_err(|_source| missing())?;
    let resolved = base_url.join(location).map_err(|_source| missing())?;
    resolved
        .query_pairs()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.into_owned())
        .ok_or_else(missing)
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
