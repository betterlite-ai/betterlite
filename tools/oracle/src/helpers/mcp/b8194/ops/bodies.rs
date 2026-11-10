use betterlite_http::user_url::httpx_url::HttpxUrl;
use betterlite_server::mcp::debug::preview::secrets::request_secret_values;
use betterlite_server::mcp::debug::preview::text::safe_upstream_url;
use betterlite_server::mcp::debug::preview::{
    describe_upstream_response, masked_headers, preview, request_body_preview,
};
use serde_json::json;

use crate::helpers::HelperResult;

use super::super::input::{DescribeInput, PreviewInput, RequestInput};
use super::{failure, mapped, redactor, upstream_request, upstream_response};

pub(super) fn previews(input: &PreviewInput) -> HelperResult {
    let redactor = redactor()?;
    let results = mapped(&input.scenarios, |scenario| {
        preview(
            scenario.body.as_bytes(),
            scenario.content_type.as_deref().unwrap_or_default(),
            &scenario.secrets,
            &redactor,
        )
    });
    Ok(json!({ "previews": results }))
}

pub(super) fn request_secrets(input: &RequestInput) -> HelperResult {
    let redactor = redactor()?;
    let mut results = Vec::with_capacity(input.scenarios.len());
    for spec in &input.scenarios {
        let request = upstream_request(spec)?;
        let secrets = request_secret_values(&request);
        results.push(json!({
            "secrets": secrets,
            "request_preview": request_body_preview(&request, secrets.as_deref(), &redactor),
            "masked_headers": masked_headers(&request.headers),
        }));
    }
    Ok(json!({ "results": results }))
}

pub(super) fn describe(input: &DescribeInput) -> HelperResult {
    let redactor = redactor()?;
    let mut urls = Vec::with_capacity(input.urls.len());
    for url in &input.urls {
        let parsed = HttpxUrl::parse(url)
            .map_err(|error| Box::new(failure("InvalidURL", error.to_string())))?;
        urls.push(safe_upstream_url(&parsed));
    }
    let mut descriptions = Vec::with_capacity(input.scenarios.len());
    for scenario in &input.scenarios {
        let request = scenario
            .request
            .as_ref()
            .map(upstream_request)
            .transpose()?;
        let response = upstream_response(&scenario.response, request)?;
        descriptions.push(describe_upstream_response(&response, &redactor));
    }
    Ok(json!({ "urls": urls, "descriptions": descriptions }))
}
