use std::collections::BTreeMap;
use std::future::ready;
use std::time::{Duration, UNIX_EPOCH};

use betterlite_bedrock::aws_endpoints::error::AwsEndpointError;
use betterlite_bedrock::aws_settings::sources::Credentials;
use betterlite_bedrock::search::error::SearchError;
use betterlite_bedrock::search::headers::validate_environment;
use betterlite_bedrock::search::request::{Params, transform_search_request};
use betterlite_bedrock::search::signing::{SignRequest, sign_request};
use betterlite_bedrock::search::url::complete_url;
use betterlite_bedrock::signing::error::SigningError;
use betterlite_bedrock::signing::request::AwsSignedRequest;
use betterlite_config::sections::bedrock_services_search::{
    GATEWAY_TOKEN_VARIABLE, GATEWAY_URL_VARIABLE,
};
use betterlite_models::decisions::Json;
use betterlite_openai::openai_base_config::Headers;
use betterlite_openai::search::provider::SearchQuery;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::Builder;

mod tests;

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        Ok(scenarios_result(input))
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "scenario", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    ConfiguredGatewayRequired { runs: Vec<GatewayRun> },
    McpRequestHeaders { runs: Vec<HeaderRun> },
    WebsearchToolSuffix { runs: Vec<ToolRun> },
    QueryListAnd200CharCap { runs: Vec<ToolRun> },
    HttpsOrLoopbackOnly { runs: Vec<SignRun> },
    GatewayBearerTrust { runs: Vec<SignRun> },
    GatewayHostSigning { runs: Vec<SignRun> },
    GatewaySigningRegion { runs: Vec<SignRun> },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GatewayRun {
    api_base: Option<String>,
    gateway_url: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HeaderRun {
    #[serde(default)]
    headers: Headers,
    protocol_version: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ToolRun {
    query: SearchQuery,
    #[serde(default)]
    optional_params: Params,
    configured_tool_name: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SignRun {
    api_base: String,
    #[serde(default)]
    headers: Headers,
    #[serde(default)]
    optional_params: Params,
    request_data: Json,
    api_key: Option<String>,
    gateway_url: Option<String>,
    gateway_token: Option<String>,
    credentials: CredentialRun,
    session_region: Option<String>,
    now: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CredentialRun {
    access_key_id: String,
    secret_access_key: String,
    session_token: Option<String>,
}

fn scenarios_result(input: &Input) -> Value {
    match input {
        Input::ConfiguredGatewayRequired { runs } => {
            json!({"results": runs.iter().map(gateway_result).collect::<Vec<_>>()})
        }
        Input::McpRequestHeaders { runs } => {
            json!({"results": runs.iter().map(header_result).collect::<Vec<_>>()})
        }
        Input::WebsearchToolSuffix { runs } | Input::QueryListAnd200CharCap { runs } => {
            json!({"results": runs.iter().map(request_result).collect::<Vec<_>>()})
        }
        Input::HttpsOrLoopbackOnly { runs }
        | Input::GatewayBearerTrust { runs }
        | Input::GatewayHostSigning { runs }
        | Input::GatewaySigningRegion { runs } => {
            json!({"results": runs.iter().map(sign_result).collect::<Vec<_>>()})
        }
    }
}

fn gateway_result(run: &GatewayRun) -> Value {
    match complete_url(run.api_base.as_deref(), run.gateway_url.as_deref()) {
        Ok(url) => json!({"api_base": run.api_base, "ok": true, "url": url}),
        Err(error) => json!({
            "api_base": run.api_base,
            "ok": false,
            "error": error_kind(&error),
        }),
    }
}

fn header_result(run: &HeaderRun) -> Value {
    let merged = validate_environment(run.headers.clone(), run.protocol_version.as_deref());
    json!({"ok": true, "headers": merged})
}

fn request_result(run: &ToolRun) -> Value {
    match transform_search_request(
        &run.query,
        &run.optional_params,
        run.configured_tool_name.as_deref(),
    ) {
        Ok(body) => json!({"ok": true, "body": body}),
        Err(error) => json!({"ok": false, "error": error_kind(&error)}),
    }
}

fn sign_result(run: &SignRun) -> Value {
    let environment = signing_environment(run);
    let api_key = run
        .api_key
        .as_ref()
        .map(|key| SecretString::from(key.clone()));
    let credentials = credentials_of(run);
    let Some(now) = UNIX_EPOCH.checked_add(Duration::from_secs(run.now)) else {
        return json!({"ok": false, "error": "invalid_clock"});
    };
    let runtime = match Builder::new_current_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(error) => return json!({"ok": false, "error": error.to_string()}),
    };
    match runtime.block_on(sign_request(
        &SignRequest {
            headers: &run.headers,
            optional_params: &run.optional_params,
            request_data: &run.request_data,
            api_base: &run.api_base,
            api_key: api_key.as_ref(),
            session_region: run.session_region.as_deref(),
            now,
        },
        &environment,
        move |_settings, _region| ready(Ok(credentials)),
    )) {
        Ok(signed) => signed_result(&signed),
        Err(error) => json!({"ok": false, "error": error_kind(&error)}),
    }
}

// the helper reads only its call input, so the case's environment travels in each run
fn signing_environment(run: &SignRun) -> BTreeMap<String, String> {
    let mut environment = BTreeMap::new();
    if let Some(url) = &run.gateway_url {
        environment.insert(GATEWAY_URL_VARIABLE.to_owned(), url.clone());
    }
    if let Some(token) = &run.gateway_token {
        environment.insert(GATEWAY_TOKEN_VARIABLE.to_owned(), token.clone());
    }
    environment
}

fn credentials_of(run: &SignRun) -> Credentials {
    Credentials {
        access_key_id: SecretString::from(run.credentials.access_key_id.clone()),
        secret_access_key: SecretString::from(run.credentials.secret_access_key.clone()),
        session_token: run
            .credentials
            .session_token
            .as_ref()
            .map(|token| SecretString::from(token.clone())),
    }
}

fn signed_result(signed: &AwsSignedRequest) -> Value {
    let Ok(body) = String::from_utf8(signed.body.clone()) else {
        return json!({"ok": false, "error": "non_utf8_body"});
    };
    json!({
        "ok": true,
        "method": signed.method,
        "url": signed.url,
        "headers": signed.headers.iter().cloned().collect::<BTreeMap<_, _>>(),
        "body": body,
    })
}

fn error_kind(error: &SearchError) -> String {
    match error {
        SearchError::GatewayUrlMissing => "gateway_url_missing".to_owned(),
        SearchError::ToolName { .. } => "tool_name".to_owned(),
        SearchError::RequestBody => "request_body".to_owned(),
        SearchError::PlaintextTransport { .. } => "plaintext_transport".to_owned(),
        SearchError::UntrustedSigningHost { .. } => "untrusted_signing_host".to_owned(),
        SearchError::SigningRegion { .. } => "signing_region".to_owned(),
        SearchError::WrongType { .. } => "wrong_type".to_owned(),
        SearchError::Parameters(_) => "parameters".to_owned(),
        SearchError::Credential(_) => "credential".to_owned(),
        SearchError::Signing(SigningError::Endpoint(AwsEndpointError::Region { .. })) => {
            "invalid_region".to_owned()
        }
        SearchError::Signing(_) => "signing".to_owned(),
    }
}
