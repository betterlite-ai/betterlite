use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::Arc;

use betterlite_guardrails::hiddenlayer::hiddenlayer_authentication::HiddenlayerAuth;
use betterlite_guardrails::hiddenlayer::hiddenlayer_authentication::detection::DetectionRequest;
use betterlite_guardrails::hiddenlayer::hiddenlayer_authentication::error::HiddenlayerError;
use betterlite_guardrails::hiddenlayer::hiddenlayer_authentication::settings::HiddenlayerCredentials;
use betterlite_guardrails::hiddenlayer::hiddenlayer_authentication::token::{
    JwtError, ReqwestJwtTransport,
};
use betterlite_guardrails::primitives::json_value::JsonValue;
use betterlite_http::proxy::base_builder;
use reqwest::header::HeaderMap;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};

use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::HelperResult;

use super::scripted::{Reply, Seen, Server};
use tokio::runtime::{Builder, Runtime};

const SAAS_HOST: &str = "api.hiddenlayer.ai";
const DETECTION_BODY: &str = r#"{"messages":[{"role":"user","content":"hello"}]}"#;
const TOKEN_PREFIX: &str = "/oauth2/token";
const V1_PATH: &str = "detection/v1/interactions";
const V2_PATH: &str = "detection/v2/request-evaluations";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    version: u8,
    saas: bool,
    #[serde(default)]
    api_id: Option<String>,
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    timeout: Option<f64>,
    #[serde(default)]
    env: BTreeMap<String, String>,
    #[serde(default)]
    tokens: Vec<Reply>,
    #[serde(default)]
    detection: Option<Vec<Reply>>,
}

pub(crate) fn run(input: &Input) -> HelperResult {
    let server = Server::start(input.tokens.clone(), input.detection.clone())?;
    let credentials = HiddenlayerCredentials {
        api_id: input.api_id.clone(),
        api_key: input.api_key.clone().map(SecretString::from),
        api_base: Some(detection_base(input, &server)),
        auth_url: Some(server.base.clone()),
        timeout: input.timeout,
    };
    let client = client(input.saas, server.port)?;
    let body: JsonValue = serde_json::from_str(DETECTION_BODY).or_value_error()?;
    let raised = match HiddenlayerAuth::new(
        &credentials,
        &input.env,
        client,
        Arc::new(ReqwestJwtTransport),
    ) {
        Ok(auth) => detect(&auth, input, &body)?,
        Err(error) => Some(classify(&error)),
    };
    let seen = server.requests();
    Ok(json!({
        "raised": raised,
        "token_requests": described(&seen, true),
        "detection_requests": described(&seen, false),
    }))
}

// the SaaS hostname earns the JWT, while the resolve in the client steers it at the local server
fn detection_base(input: &Input, server: &Server) -> String {
    if input.saas {
        format!("http://{SAAS_HOST}:{}", server.port)
    } else {
        server.base.clone()
    }
}

fn client(saas: bool, port: u16) -> Result<reqwest::Client, Box<HelperError>> {
    let mut builder = base_builder();
    if saas {
        builder = builder.resolve(SAAS_HOST, SocketAddr::from(([127, 0, 0, 1], port)));
    }
    builder.build().or_config_error()
}

fn detect(
    auth: &HiddenlayerAuth,
    input: &Input,
    body: &JsonValue,
) -> Result<Option<Value>, Box<HelperError>> {
    if input.detection.is_none() {
        return Ok(None);
    }
    let request = DetectionRequest {
        path: path(input.version),
        headers: HeaderMap::new(),
        body,
    };
    match runtime()?.block_on(auth.post_detection(&request)) {
        Ok(_response) => Ok(None),
        Err(error) => Ok(Some(classify(&error))),
    }
}

const fn path(version: u8) -> &'static str {
    if version == 1 { V1_PATH } else { V2_PATH }
}

fn described(seen: &[Seen], tokens: bool) -> Vec<Value> {
    seen.iter()
        .filter(|entry| entry.path.starts_with(TOKEN_PREFIX) == tokens)
        .map(|entry| {
            json!({
                "method": entry.method,
                "path": entry.path,
                "authorization": entry.authorization,
            })
        })
        .collect()
}

fn classify(error: &HiddenlayerError) -> Value {
    match error {
        HiddenlayerError::MissingApiId => {
            json!({"kind": "missing_api_id", "message": error.to_string()})
        }
        HiddenlayerError::MissingApiKey => {
            json!({"kind": "missing_api_key", "message": error.to_string()})
        }
        HiddenlayerError::InvalidTimeout { .. } => {
            json!({"kind": "invalid_timeout", "message": error.to_string()})
        }
        HiddenlayerError::Jwt(JwtError::Status { .. }) => {
            json!({"kind": "token_status", "message": error.to_string()})
        }
        HiddenlayerError::Jwt(JwtError::Transport { source }) if source.is_timeout() => {
            json!({"kind": "timeout"})
        }
        HiddenlayerError::Jwt(JwtError::Transport { .. }) => json!({"kind": "token_transport"}),
        HiddenlayerError::Jwt(JwtError::InvalidResponse { .. }) => {
            json!({"kind": "invalid_response"})
        }
        HiddenlayerError::Jwt(JwtError::Decode { .. }) => json!({"kind": "token_decode"}),
        HiddenlayerError::Jwt(JwtError::Body { .. } | JwtError::TooLarge { .. }) => {
            json!({"kind": "token_body"})
        }
        HiddenlayerError::Transport { .. } => json!({"kind": "detection_transport"}),
        HiddenlayerError::Body { .. } => json!({"kind": "detection_body"}),
        HiddenlayerError::Status { status, .. } => {
            json!({"kind": "detection_status", "status": status})
        }
        HiddenlayerError::RefreshTask { .. } => json!({"kind": "refresh_task"}),
    }
}

fn runtime() -> Result<Runtime, Box<HelperError>> {
    Builder::new_current_thread()
        .enable_all()
        .build()
        .or_config_error()
}
