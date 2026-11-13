use betterlite_context::aws_partition::get_aws_partition;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BedrockEndpointType {
    #[default]
    Runtime,
    Agent,
    Agentcore,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeEndpoint {
    pub endpoint_url: String,
    pub proxy_endpoint_url: String,
}

#[derive(Debug, Clone, Copy)]
pub struct RuntimeEndpointInput<'text> {
    pub api_base: Option<&'text str>,
    pub configured_endpoint: Option<&'text str>,
    pub environment_endpoint: Option<&'text str>,
    pub region: &'text str,
    pub endpoint_type: BedrockEndpointType,
}

#[must_use]
pub fn default_endpoint_url(endpoint_type: BedrockEndpointType, region: &str) -> String {
    let service = match endpoint_type {
        BedrockEndpointType::Runtime => "bedrock-runtime",
        BedrockEndpointType::Agent => "bedrock-agent-runtime",
        BedrockEndpointType::Agentcore => "bedrock-agentcore",
    };
    let dns_suffix = get_aws_partition(Some(region)).dns_suffix;
    format!("https://{service}.{region}.{dns_suffix}")
}

#[must_use]
pub fn runtime_endpoint(input: RuntimeEndpointInput<'_>) -> RuntimeEndpoint {
    let environment = input.environment_endpoint.filter(|value| !value.is_empty());
    let endpoint_url = input
        .api_base
        .map(str::to_owned)
        .or_else(|| input.configured_endpoint.map(str::to_owned))
        .or_else(|| environment.map(str::to_owned))
        .unwrap_or_else(|| default_endpoint_url(input.endpoint_type, input.region));
    let proxy_endpoint_url = input
        .configured_endpoint
        .map(str::to_owned)
        .or_else(|| environment.map(str::to_owned))
        .unwrap_or_else(|| endpoint_url.clone());
    RuntimeEndpoint {
        endpoint_url,
        proxy_endpoint_url,
    }
}
