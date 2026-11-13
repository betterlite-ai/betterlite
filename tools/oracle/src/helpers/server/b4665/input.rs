use betterlite_mcp::credential_contracts::auth_config::oauth::{
    DEFAULT_SUBJECT_TOKEN_TYPE, TokenExchangeConfig, TokenExchangeProfile,
};
use betterlite_mcp::credential_contracts::header_carrier::HeaderCarrier;
use betterlite_mcp::credential_contracts::token_endpoint_auth::TokenEndpointAuthMethod;
use betterlite_server::mcp::outbound_credentials::token_exchanger::ExchangeBody;
use secrecy::SecretString;
use serde::Deserialize;

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ProfileSpec {
    Rfc8693,
    EntraObo,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum AuthMethodSpec {
    ClientSecretBasic,
    ClientSecretPost,
}

#[derive(Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum StepOp {
    Exchange,
    Invalidate,
    Concurrent,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ResponseKind {
    Body,
    Transport,
    SubjectRejected,
    ClientFault,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ResponseSpec {
    pub(super) kind: ResponseKind,
    #[serde(default)]
    pub(super) body: Option<ExchangeBody>,
    #[serde(default)]
    pub(super) detail: Option<String>,
    #[serde(default)]
    pub(super) claims: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ConfigSpec {
    #[serde(default)]
    profile: Option<ProfileSpec>,
    #[serde(default)]
    subject_token_type: Option<String>,
    #[serde(default)]
    token_exchange_endpoint: Option<String>,
    #[serde(default)]
    audience: Option<String>,
    #[serde(default)]
    client_id: Option<String>,
    #[serde(default)]
    client_secret: Option<String>,
    #[serde(default)]
    token_endpoint_auth_method: Option<AuthMethodSpec>,
    #[serde(default)]
    scopes: Option<Vec<String>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StepSpec {
    pub(super) op: StepOp,
    pub(super) subject_token: String,
    #[serde(default)]
    pub(super) tenant_id: Option<String>,
    #[serde(default)]
    pub(super) now_ms: Option<i64>,
    #[serde(default)]
    pub(super) config: Option<ConfigSpec>,
    #[serde(default)]
    pub(super) responses: Vec<ResponseSpec>,
    #[serde(default)]
    pub(super) count: Option<usize>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(super) config: ConfigSpec,
    #[serde(default)]
    pub(super) default_ttl_ms: Option<u64>,
    #[serde(default)]
    pub(super) min_ttl_ms: Option<u64>,
    #[serde(default)]
    pub(super) expiry_buffer_ms: Option<u64>,
    pub(super) steps: Vec<StepSpec>,
}

impl StepOp {
    pub(super) const fn as_str(self) -> &'static str {
        match self {
            Self::Exchange => "exchange",
            Self::Invalidate => "invalidate",
            Self::Concurrent => "concurrent",
        }
    }
}

pub(super) fn token_config(spec: &ConfigSpec) -> TokenExchangeConfig {
    let client_secret = spec
        .client_secret
        .as_ref()
        .map(|secret| SecretString::from(secret.clone()));
    TokenExchangeConfig {
        header: HeaderCarrier::default(),
        profile: match spec.profile {
            Some(ProfileSpec::EntraObo) => TokenExchangeProfile::EntraObo,
            Some(ProfileSpec::Rfc8693) | None => TokenExchangeProfile::Rfc8693,
        },
        subject_token_type: spec
            .subject_token_type
            .clone()
            .unwrap_or_else(|| DEFAULT_SUBJECT_TOKEN_TYPE.to_owned()),
        token_exchange_endpoint: spec.token_exchange_endpoint.clone(),
        audience: spec.audience.clone(),
        client_id: spec.client_id.clone(),
        client_secret,
        token_endpoint_auth_method: spec.token_endpoint_auth_method.map(|method| match method {
            AuthMethodSpec::ClientSecretBasic => TokenEndpointAuthMethod::ClientSecretBasic,
            AuthMethodSpec::ClientSecretPost => TokenEndpointAuthMethod::ClientSecretPost,
        }),
        scopes: spec.scopes.clone().unwrap_or_default(),
    }
}
