use std::collections::BTreeMap;
use std::sync::{Arc, PoisonError, RwLock};
use std::time::{Duration, TryFromFloatSecsError};

use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::opaque::OpaqueParams;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use reqwest::Client;
use serde::de::DeserializeOwned;
use thiserror::Error;

use crate::mcp_jwt_signer::signer_settings::SignerOptions;
use crate::mcp_jwt_signer::{JwtSigner, JwtSignerError};

mod tests;

pub const WIRE_NAME: &str = "mcp_jwt_signer";
pub const SIGNER_TYPE_NAME: &str = "MCPJWTSigner";
const PRE_MCP_CALL: &str = "pre_mcp_call";
const OPTIONAL_PARAMS: &str = "optional_params";

#[derive(Default)]
pub struct JwtSignerSlot {
    signer: RwLock<Option<Arc<JwtSigner>>>,
}

impl JwtSignerSlot {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            signer: RwLock::new(None),
        }
    }

    #[must_use]
    pub fn get(&self) -> Option<Arc<JwtSigner>> {
        self.signer
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub fn set(&self, signer: Arc<JwtSigner>) {
        let previous = self
            .signer
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .replace(signer);
        if previous.is_some() {
            tracing::warn!(
                "MCPJWTSigner: replacing the existing signer, so tokens signed with the old key fail JWKS verification. Avoid configuring multiple mcp_jwt_signer guardrails."
            );
        }
    }
}

// LiteLLM's signer constructor reads the process environment and builds its own HTTP client; betterlite injects both through this port
pub trait CallbackRegistry {
    fn add_litellm_callback(&mut self, signer: Arc<JwtSigner>);
    fn environment(&self) -> &dyn EnvironmentLookup;
    fn http_client(&self) -> Client;
}

#[derive(Debug, Error)]
pub enum GuardrailInitError {
    #[error("MCPJWTSigner guardrail requires a guardrail_name")]
    MissingGuardrailName,
    #[error(
        "MCPJWTSigner guardrail '{guardrail_name}' has mode='{mode}' but must use mode='pre_mcp_call'. JWT injection only fires for MCP tool calls."
    )]
    WrongMode {
        guardrail_name: String,
        mode: String,
    },
    #[error("MCPJWTSigner option {field} has an invalid value")]
    InvalidOption {
        field: &'static str,
        #[source]
        source: serde_json::Error,
    },
    #[error("MCPJWTSigner option optional_params has an invalid value")]
    InvalidOptionalParams,
    #[error("MCPJWTSigner option timeout has an invalid value")]
    InvalidTimeout {
        #[source]
        source: TryFromFloatSecsError,
    },
    #[error(transparent)]
    Signer(#[from] JwtSignerError),
}

pub fn initialize_mcp_jwt_signer(
    params: &LitellmParams,
    guardrail_name: Option<&str>,
    registry: &mut dyn CallbackRegistry,
    slot: &JwtSignerSlot,
) -> Result<Arc<JwtSigner>, GuardrailInitError> {
    let Some(name) = guardrail_name.filter(|entry_name| !entry_name.is_empty()) else {
        return Err(GuardrailInitError::MissingGuardrailName);
    };
    let pre_mcp_call = matches!(
        &params.mode,
        LitellmParamsMode::One(mode) if mode.as_str() == PRE_MCP_CALL
    );
    if !pre_mcp_call {
        return Err(GuardrailInitError::WrongMode {
            guardrail_name: name.to_owned(),
            mode: mode_text(&params.mode),
        });
    }
    let options = signer_options(params)?;
    let signer = Arc::new(JwtSigner::new(
        &options,
        registry.environment(),
        registry.http_client(),
    )?);
    slot.set(Arc::clone(&signer));
    registry.add_litellm_callback(Arc::clone(&signer));
    Ok(signer)
}

fn signer_options(params: &LitellmParams) -> Result<SignerOptions, GuardrailInitError> {
    let optional = optional_params(params)?;
    let extra = &params.base.extra;
    Ok(SignerOptions {
        issuer: signer_option(optional, extra, "issuer")?,
        audience: signer_option(optional, extra, "audience")?,
        ttl_seconds: signer_option(optional, extra, "ttl_seconds")?,
        access_token_discovery_uri: signer_option(optional, extra, "access_token_discovery_uri")?,
        token_introspection_endpoint: signer_option(
            optional,
            extra,
            "token_introspection_endpoint",
        )?,
        verify_issuer: signer_option(optional, extra, "verify_issuer")?,
        verify_audience: signer_option(optional, extra, "verify_audience")?,
        end_user_claim_sources: signer_option(optional, extra, "end_user_claim_sources")?,
        add_claims: signer_option(optional, extra, "add_claims")?,
        set_claims: signer_option(optional, extra, "set_claims")?,
        remove_claims: signer_option(optional, extra, "remove_claims")?,
        channel_token_audience: signer_option(optional, extra, "channel_token_audience")?,
        channel_token_ttl: signer_option(optional, extra, "channel_token_ttl")?,
        required_claims: signer_option(optional, extra, "required_claims")?,
        optional_claims: signer_option(optional, extra, "optional_claims")?,
        debug_headers: signer_option(optional, extra, "debug_headers")?.unwrap_or(false),
        allowed_scopes: signer_option(optional, extra, "allowed_scopes")?,
        timeout: signer_timeout(params)?,
    })
}

fn optional_params(
    params: &LitellmParams,
) -> Result<Option<&BTreeMap<String, GuardrailValue>>, GuardrailInitError> {
    match params.base.extra.0.get(OPTIONAL_PARAMS) {
        None | Some(GuardrailValue::Null) => Ok(None),
        Some(GuardrailValue::Object(map)) => Ok(Some(map)),
        Some(_wrong) => Err(GuardrailInitError::InvalidOptionalParams),
    }
}

fn signer_option<T>(
    optional: Option<&BTreeMap<String, GuardrailValue>>,
    extra: &OpaqueParams,
    key: &'static str,
) -> Result<Option<T>, GuardrailInitError>
where
    T: DeserializeOwned,
{
    if let Some(value) = optional.and_then(|map| map.get(key))
        && !matches!(value, GuardrailValue::Null)
    {
        return decode_option(key, value);
    }
    match extra.0.get(key) {
        None | Some(GuardrailValue::Null) => Ok(None),
        Some(value) => decode_option(key, value),
    }
}

fn decode_option<T>(
    key: &'static str,
    value: &GuardrailValue,
) -> Result<Option<T>, GuardrailInitError>
where
    T: DeserializeOwned,
{
    let encoded = serde_json::to_vec(value)
        .map_err(|source| GuardrailInitError::InvalidOption { field: key, source })?;
    serde_json::from_slice(&encoded)
        .map(Some)
        .map_err(|source| GuardrailInitError::InvalidOption { field: key, source })
}

fn signer_timeout(params: &LitellmParams) -> Result<Option<Duration>, GuardrailInitError> {
    let Some(seconds) = params.base.fields.timeout else {
        return Ok(None);
    };
    let timeout = Duration::try_from_secs_f64(seconds)
        .map_err(|source| GuardrailInitError::InvalidTimeout { source })?;
    Ok(Some(timeout))
}

fn mode_text(mode: &LitellmParamsMode) -> String {
    match mode {
        LitellmParamsMode::One(text) => text.clone(),
        LitellmParamsMode::Many(items) => serde_json::to_string(items).unwrap_or_default(),
        LitellmParamsMode::Tagged(tagged) => serde_json::to_string(tagged).unwrap_or_default(),
    }
}
