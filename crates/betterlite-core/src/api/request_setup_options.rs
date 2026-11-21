pub mod callbacks;
pub mod destinations;
pub mod error;
pub mod logging;
pub mod session;

mod tests;

use betterlite_anthropic::feature_flags::billing::{CLAUDE_CLI_PREFIX, CLAUDE_CODE_PREFIX};
use betterlite_auth::auxiliary_routes::MCP_ROUTES;
use betterlite_auth::route_match::classifier::is_llm_api_route;
use betterlite_auth::route_match::templates::{RouteValue, check_route_access};
use betterlite_config::sections::provider_url_destination::pattern::AllowedDestination;
use betterlite_config::sections::request_metadata_safety::MissingSessionIdPolicy;
use betterlite_models::decisions::Json;
use betterlite_models::provider_defaults_a::environment::EnvironmentLookup;
use uuid::Uuid;

use crate::api::request_destination::RequestData;
use crate::api::request_metadata_safety::metadata::MetadataField;
use crate::api::request_principal_metadata::team_callbacks::DefaultTeamSettings;
use crate::api::request_principal_metadata::{Metadata, PrincipalFacts};
use crate::api::request_setup_options::callbacks::CallbackSettings;
use crate::api::request_setup_options::error::RequestSetupOptionsError;
use crate::api::request_setup_options::session::missing_session_stage;

const API_VERSION_QUERY: &str = "api-version";
const API_VERSION_KEY: &str = "api_version";
const DROP_PARAMS_KEY: &str = "drop_params";
const CODEX_PREFIX: &str = "codex";

pub trait SessionIdSource {
    fn session_id(&self) -> String;
}

pub struct RandomSessionIds;

impl SessionIdSource for RandomSessionIds {
    fn session_id(&self) -> String {
        Uuid::new_v4().to_string()
    }
}

// decrypt_callback_vars: the stored metadata with its callback_vars values decrypted, plaintext rows unchanged
pub trait CallbackVarsDecryption {
    fn decrypt(&self, metadata: &Metadata) -> Metadata;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallbackSource {
    Key,
    Team,
    OtelDestination,
}

impl CallbackSource {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Key => "key-level",
            Self::Team => "team-level",
            Self::OtelDestination => "otel-destination",
        }
    }
}

// the warnings LiteLLM logs and keeps going after, in the order they were logged
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SetupDiagnostic {
    UnknownSessionPolicy { policy: String },
    InvalidCallbackMetadata { source: CallbackSource },
}

#[derive(Debug, Clone)]
pub struct RequestSetupSettings {
    pub callback_disabling: bool,
    pub missing_session_id: Option<MissingSessionIdPolicy>,
    pub otel_v2_enabled: bool,
    pub default_team_settings: Vec<DefaultTeamSettings>,
    pub allowed_hosts: Vec<AllowedDestination>,
}

pub struct RequestSetupOptions<Ids, Decryption, Environment> {
    pub settings: RequestSetupSettings,
    pub session_ids: Ids,
    pub decryption: Decryption,
    pub environment: Environment,
}

#[derive(Debug, Clone)]
pub struct RequestSetupOptionsInput {
    pub data: RequestData,
    pub route: String,
    pub query: Vec<(String, String)>,
    pub user_agent: String,
    pub metadata_variable_name: MetadataField,
    pub principal: PrincipalFacts,
}

#[derive(Debug, Clone)]
pub struct RequestSetupOptionsResult {
    pub data: RequestData,
    pub callback_settings: Option<CallbackSettings>,
    pub diagnostics: Vec<SetupDiagnostic>,
}

impl<Ids, Decryption, Environment> RequestSetupOptions<Ids, Decryption, Environment>
where
    Ids: SessionIdSource,
    Decryption: CallbackVarsDecryption,
    Environment: EnvironmentLookup,
{
    pub fn prepare(
        &self,
        input: RequestSetupOptionsInput,
    ) -> Result<RequestSetupOptionsResult, RequestSetupOptionsError> {
        let mut data = input.data;
        // dict(request.query_params) keeps the last value of a repeated name
        if let Some((_, version)) = input
            .query
            .iter()
            .rev()
            .find(|(name, _)| name == API_VERSION_QUERY)
        {
            data.insert(API_VERSION_KEY.to_owned(), Json::Text(version.clone()));
        }
        let route = input.route.as_str();
        let mut diagnostics = Vec::new();
        diagnostics.extend(missing_session_stage(
            &mut data,
            input.metadata_variable_name,
            self.settings.missing_session_id.as_ref(),
            is_llm_api_route(&RouteValue::Text(route)) && !check_route_access(route, MCP_ROUTES),
            &self.session_ids,
        )?);
        // LiteLLM's litellm_settings.drop_params check expects a dict where ProxyConfig keeps a MappingProxyType, so only the caller decides
        if is_agentic_cli(&input.user_agent)
            && matches!(data.get(DROP_PARAMS_KEY), None | Some(Json::Null))
        {
            data.insert(DROP_PARAMS_KEY.to_owned(), Json::Bool(true));
        }
        let callback_settings =
            self.dynamic_logging_settings(&input.principal, &mut diagnostics)?;
        Ok(RequestSetupOptionsResult {
            data,
            callback_settings,
            diagnostics,
        })
    }
}

// is_claude_code_user_agent or is_codex_user_agent; the codex check in identity.rs is private to that module
fn is_agentic_cli(user_agent: &str) -> bool {
    user_agent.starts_with(CLAUDE_CLI_PREFIX)
        || user_agent.starts_with(CLAUDE_CODE_PREFIX)
        || (user_agent
            .get(..CODEX_PREFIX.len())
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(CODEX_PREFIX))
            && matches!(
                user_agent.as_bytes().get(CODEX_PREFIX.len()),
                Some(b'-' | b'_' | b' ' | b'/')
            ))
}
