use std::str::from_utf8;

use betterlite_http::user_url::httpx_url::HttpxUrl;
use betterlite_redact::secret::Redactor;
use betterlite_server::mcp::debug::preview::upstream::{
    BodyState, UpstreamRequest, UpstreamResponse, capture_error_response,
};
use futures_util::stream;
use serde_json::{Value, json};
use tokio::runtime::Runtime;

use crate::helpers::{HelperError, HelperResult};

use super::super::input::{CaptureInput, CaptureScenario};
use super::{body_state, failure, header_map, redactor, runtime, status_code};

pub(super) fn capture(input: &CaptureInput) -> HelperResult {
    let redactor = redactor()?;
    let runtime = runtime()?;
    let mut results = Vec::with_capacity(input.scenarios.len());
    for scenario in &input.scenarios {
        results.push(capture_scenario(scenario, &runtime, &redactor)?);
    }
    Ok(json!({ "results": results }))
}

fn capture_scenario(
    scenario: &CaptureScenario,
    runtime: &Runtime,
    redactor: &Redactor,
) -> Result<Value, Box<HelperError>> {
    let request = match scenario.request.as_ref() {
        Some(spec) => Some(UpstreamRequest {
            method: spec.method.clone(),
            url: HttpxUrl::parse(&spec.url)
                .map_err(|error| Box::new(failure("InvalidURL", error.to_string())))?,
            headers: header_map(&spec.headers)?,
            body: body_state(spec.body.as_ref()),
        }),
        None => None,
    };
    let status = status_code(scenario.status)?;
    let mut response = UpstreamResponse {
        status,
        headers: header_map(&scenario.headers)?,
        body: BodyState::NotRead,
        request,
        captured: None,
    };
    let body = scenario.body.as_bytes().to_vec();
    let chunk_size = scenario.chunk_size.unwrap_or_else(|| body.len().max(1));
    let chunks = chunk_results(&body, chunk_size, scenario.fail_after_chunks);
    runtime.block_on(capture_error_response(
        &mut response,
        stream::iter(chunks),
        redactor,
    ));
    let captured = response.captured.clone();
    let content = match &response.body {
        BodyState::Read(raw) => Some(from_utf8(raw).map(str::to_owned).unwrap_or_default()),
        BodyState::NotRead => None,
    };
    Ok(json!({ "captured": captured, "body": content }))
}

struct StreamBroken;

fn chunk_results(
    body: &[u8],
    chunk_size: usize,
    fail_after: Option<usize>,
) -> Vec<Result<Vec<u8>, StreamBroken>> {
    let mut results = Vec::new();
    let mut offset = 0_usize;
    while offset < body.len() {
        if fail_after.is_some_and(|limit| results.len() >= limit) {
            results.push(Err(StreamBroken));
            return results;
        }
        let end = offset.saturating_add(chunk_size).min(body.len());
        results.push(Ok(body.get(offset..end).unwrap_or_default().to_vec()));
        offset = end;
    }
    results
}
