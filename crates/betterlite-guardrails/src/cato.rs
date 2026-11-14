use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::tls::{TlsPolicy, Verification, VerifySetting};
use secrecy::SecretString;

pub mod actions;
pub mod error;
pub mod headers;
pub mod inspection;
pub mod redaction;
pub mod schema_strings;
mod tests;
mod tls;
mod transport;
pub mod wire;

use error::CatoError;
use wire::AnalyzeRequest;

pub const DEFAULT_API_BASE: &str = "https://api.aisec.catonetworks.com";
pub const LITELLM_VERSION: &str = "1.105.1";
pub const DEFAULT_BLOCK_MESSAGE: &str = "the request was blocked by the Cato Networks guardrail";
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(600);
pub(crate) const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

pub const SUPPORTED_EVENT_HOOKS: [EventHook; 3] = [
    EventHook::PreCall,
    EventHook::DuringCall,
    EventHook::PostCall,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallHook {
    PreCall,
    Moderation,
    Output,
}

impl CallHook {
    #[must_use]
    pub const fn header_value(self) -> &'static str {
        match self {
            Self::PreCall => "pre_call",
            Self::Moderation => "moderation",
            Self::Output => "output",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WsSsl {
    Disabled,
    SystemRoots,
    CaFile(String),
}

#[derive(Debug, Clone, Default)]
pub struct CatoParams {
    pub guardrail_name: Option<String>,
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub inspect_embeddings: Option<bool>,
    pub ssl_verify: Option<VerifySetting>,
    pub timeout: Option<f64>,
}

#[derive(Debug)]
pub struct CatoGuardrail {
    pub guardrail_name: Option<String>,
    api_key: SecretString,
    pub api_base: String,
    pub ws_api_base: String,
    pub inspect_embeddings: bool,
    pub ws_ssl: Option<WsSsl>,
    pub timeout: Duration,
    pub(crate) client: reqwest::Client,
}

impl CatoGuardrail {
    pub fn new<L>(params: CatoParams, lookup: &L) -> Result<Self, CatoError>
    where
        L: EnvironmentLookup,
    {
        let environment_key = lookup
            .lookup("CATO_API_KEY")
            .filter(|key| !key.is_empty())
            .map(SecretString::from);
        let api_key = params
            .api_key
            .or(environment_key)
            .ok_or(CatoError::MissingApiKey)?;
        let api_base = params
            .api_base
            .or_else(|| {
                lookup
                    .lookup("CATO_API_BASE")
                    .filter(|base| !base.is_empty())
            })
            .unwrap_or_else(|| DEFAULT_API_BASE.to_owned());
        let api_base = api_base.trim_end_matches('/').to_owned();
        let ws_api_base = api_base
            .replace("http://", "ws://")
            .replace("https://", "wss://");
        let ws_ssl = tls::resolve_ws_ssl(params.ssl_verify.as_ref(), &ws_api_base, lookup);
        let timeout = match params.timeout {
            Some(seconds) => Duration::try_from_secs_f64(seconds)
                .map_err(|source| CatoError::TimeoutSetting { seconds, source })?,
            None => DEFAULT_TIMEOUT,
        };
        let client = build_client(params.ssl_verify.as_ref(), lookup, timeout)?;
        Ok(Self {
            guardrail_name: params.guardrail_name,
            api_key,
            api_base,
            ws_api_base,
            inspect_embeddings: params.inspect_embeddings.unwrap_or(false),
            ws_ssl,
            timeout,
            client,
        })
    }

    #[must_use]
    pub fn api_key(&self) -> &SecretString {
        &self.api_key
    }

    pub async fn call(
        &self,
        data: &mut BTreeMap<String, GuardrailValue>,
        hook: CallHook,
        key_alias: Option<&str>,
        user_email: Option<&str>,
    ) -> Result<(), CatoError> {
        let call_id = match data.get("litellm_call_id") {
            Some(GuardrailValue::Text(call_id)) => Some(call_id.as_str()),
            _ => None,
        };
        let headers = headers::build_cato_headers(self, hook, key_alias, user_email, call_id)?;
        let request = AnalyzeRequest {
            messages: inspection::inspection_messages(data),
        };
        let response = self.post_analyze(&request, headers).await?;
        actions::handle_actions(&response, data)
    }
}

fn build_client<L>(
    ssl_verify: Option<&VerifySetting>,
    lookup: &L,
    timeout: Duration,
) -> Result<reqwest::Client, CatoError>
where
    L: EnvironmentLookup,
{
    let verification = tls::resolve_verification(ssl_verify, lookup);
    let policy = TlsPolicy {
        ca_file: match &verification {
            Verification::CaFile(path) => Some(path.clone()),
            Verification::Enabled | Verification::Disabled => None,
        },
        verification,
        certificate: None,
        http2: false,
        ecdh_curve: None,
        user_agent: format!("litellm/{LITELLM_VERSION}"),
    };
    betterlite_http::tls::apply_policy(betterlite_http::proxy::base_builder(), &policy)
        .map_err(|source| CatoError::Tls { source })?
        .timeout(timeout)
        .connect_timeout(timeout.min(Duration::from_secs(5)))
        .build()
        .map_err(|source| CatoError::Client { source })
}
