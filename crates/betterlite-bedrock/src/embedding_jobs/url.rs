use betterlite_http::url_path::quote_segment;

use crate::aws_endpoints::runtime::{BedrockEndpointType, RuntimeEndpointInput, runtime_endpoint};

#[derive(Debug, Clone, Copy)]
pub struct StatusEndpoint<'text> {
    pub configured_endpoint: Option<&'text str>,
    pub environment_endpoint: Option<&'text str>,
    pub region: &'text str,
}

#[must_use]
pub fn async_invoke_status_url(endpoint: StatusEndpoint<'_>, invocation_arn: &str) -> String {
    let runtime = runtime_endpoint(RuntimeEndpointInput {
        api_base: None,
        configured_endpoint: endpoint.configured_endpoint,
        environment_endpoint: endpoint.environment_endpoint,
        region: endpoint.region,
        endpoint_type: BedrockEndpointType::Runtime,
    });
    // the proxy endpoint only routes proxy traffic; the status request uses the runtime endpoint
    format!(
        "{}/async-invoke/{}",
        runtime.endpoint_url.trim_end_matches('/'),
        quote_segment(invocation_arn)
    )
}
