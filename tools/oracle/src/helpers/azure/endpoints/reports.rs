use betterlite_azure::azure_urls::EndpointSelection;
use betterlite_azure::endpoints::cloudflare::{CloudflareClient, CloudflareOutcome};
use betterlite_azure::endpoints::projection::{
    AzureClientInputs, ClientIdentity, ClientInputs, ClientKind, ClientOutcome, ClientSource,
    HttpClient, OpenAiV1ClientInputs, V1Key,
};
use secrecy::ExposeSecret;
use serde_json::{Value, json};

use super::fixture::Sources;

pub(super) fn initialize_report(
    inputs: &AzureClientInputs<'_>,
    provider_was_passed: bool,
    sources: &Sources,
) -> Value {
    let calls = sources.calls();
    json!({
        "api_version": &inputs.api_version,
        "max_retries": inputs.max_retries,
        "timeout": inputs.timeout,
        "http_client": http_json(inputs.http_client),
        "endpoint": endpoint_json(&inputs.endpoint),
        "api_key": inputs.api_key.as_ref().map(ExposeSecret::expose_secret),
        "azure_ad_token": inputs.azure_ad_token.as_ref().map(ExposeSecret::expose_secret),
        "provider": provider_kind(provider_was_passed, &calls),
        "calls": calls,
    })
}

pub(super) fn client_report(
    outcome: &ClientOutcome<'_>,
    cache_owned: Option<bool>,
    calls: &[Value],
) -> Value {
    json!({
        "source": source_name(outcome.source),
        "kind": kind_name(outcome.kind),
        "custom_query": &outcome.custom_query,
        "cache_owned": cache_owned,
        "identity": identity_json(&outcome.identity),
        "inputs": inputs_json(outcome.inputs.as_ref()),
        "calls": calls,
    })
}

pub(super) fn cloudflare_report(outcome: &CloudflareOutcome<'_>, calls: &[Value]) -> Value {
    match outcome {
        CloudflareOutcome::Supplied(client) => json!({
            "kind": "supplied",
            "supplied": {
                "kind": kind_name(client.kind),
                "custom_query": &client.custom_query,
            },
            "client": Value::Null,
            "calls": calls,
        }),
        CloudflareOutcome::Constructed(client) => json!({
            "kind": "constructed",
            "supplied": Value::Null,
            "client": constructed_json(client),
            "calls": calls,
        }),
    }
}

fn constructed_json(client: &CloudflareClient<'_>) -> Value {
    json!({
        "base_url": &client.base_url,
        "api_version": &client.api_version,
        "api_key": client.api_key.as_ref().map(ExposeSecret::expose_secret),
        "azure_ad_token": client.azure_ad_token.as_ref().map(ExposeSecret::expose_secret),
        "token_provider": client.token_provider.is_some(),
        "max_retries": client.max_retries,
        "timeout": client.timeout,
        "shared_session": client.shared_session,
        "kind": kind_name(client.kind),
    })
}

fn inputs_json(inputs: Option<&ClientInputs<'_>>) -> Value {
    match inputs {
        Some(ClientInputs::Azure(azure)) => json!({"azure": azure_json(azure)}),
        Some(ClientInputs::OpenAiV1(v1)) => json!({"open_ai_v1": v1_json(v1)}),
        None => Value::Null,
    }
}

fn azure_json(inputs: &AzureClientInputs<'_>) -> Value {
    json!({
        "endpoint": endpoint_json(&inputs.endpoint),
        "api_version": &inputs.api_version,
        "api_key": inputs.api_key.as_ref().map(ExposeSecret::expose_secret),
        "azure_ad_token": inputs.azure_ad_token.as_ref().map(ExposeSecret::expose_secret),
        "token_provider": inputs.token_provider.is_some(),
        "max_retries": inputs.max_retries,
        "timeout": inputs.timeout,
        "http_client": http_json(inputs.http_client),
    })
}

fn v1_json(inputs: &OpenAiV1ClientInputs<'_>) -> Value {
    json!({
        "key": key_json(inputs.key.as_ref()),
        "base_url": &inputs.base_url,
        "timeout": inputs.timeout,
        "max_retries": inputs.max_retries,
        "http_client": http_json(inputs.http_client),
    })
}

fn key_json(key: Option<&V1Key<'_>>) -> Value {
    match key {
        Some(V1Key::Static(_)) => json!({"kind": "static", "offload": false}),
        Some(V1Key::Provider { offload, .. }) => json!({"kind": "provider", "offload": offload}),
        None => json!({"kind": "none", "offload": false}),
    }
}

fn identity_json(identity: &ClientIdentity) -> Value {
    json!({
        "is_async": identity.is_async,
        "azure_ad_token": &identity.hashed_ad_token,
        "azure_ad_token_provider": &identity.provider_identity,
    })
}

fn endpoint_json(endpoint: &EndpointSelection) -> Value {
    json!({
        "azure_endpoint": &endpoint.azure_endpoint,
        "base_url": &endpoint.base_url,
    })
}

fn http_json(http: HttpClient) -> Value {
    json!({"is_async": http.is_async, "shared": http.shared})
}

fn provider_kind(provider_was_passed: bool, calls: &[Value]) -> &'static str {
    if provider_was_passed {
        return "passed";
    }
    for call in calls {
        match call.get("source").and_then(Value::as_str) {
            Some("client_secret") => return "client_secret",
            Some("username_password") => return "username_password",
            Some("refresh") => return "refresh",
            _ => {}
        }
    }
    "none"
}

pub(super) const fn kind_name(kind: ClientKind) -> &'static str {
    match kind {
        ClientKind::AzureOpenAi => "azure",
        ClientKind::AsyncAzureOpenAi => "async_azure",
        ClientKind::OpenAi => "openai",
        ClientKind::AsyncOpenAi => "async_openai",
        ClientKind::Other => "other",
    }
}

const fn source_name(source: ClientSource) -> &'static str {
    match source {
        ClientSource::Supplied => "supplied",
        ClientSource::Cached => "cached",
        ClientSource::Constructed => "constructed",
    }
}
