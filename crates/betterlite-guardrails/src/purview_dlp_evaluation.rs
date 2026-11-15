use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use secrecy::SecretString;
use uuid::Uuid;

use crate::purview_graph_client::{
    CacheState, DEFAULT_APP_NAME, DEFAULT_USER_ID_FIELD, GRAPH_API_BASE, LOGIN_AUTHORITY,
    PurviewGuardrailBase,
};

pub mod completion_text;
pub mod content_check;
pub mod error_handling;
pub mod hook_methods;
pub mod responses_extraction;
pub mod responses_input;
pub mod stream_assembly;
pub mod user_id_blocking;

mod test_support;

pub const GUARDRAIL_PROVIDER: &str = "microsoft_purview";
// httpx's default for the cached guardrail client, which LiteLLM keeps when no timeout is configured
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(600);

#[derive(Debug, Clone)]
pub struct PurviewDlpGuardrailParams {
    pub guardrail_name: String,
    pub tenant_id: String,
    pub client_id: String,
    pub client_secret: SecretString,
    pub purview_app_name: Option<String>,
    pub user_id_field: Option<String>,
    pub timeout: Option<Duration>,
}

pub struct MicrosoftPurviewDLPGuardrail {
    pub base: PurviewGuardrailBase,
    pub guardrail_name: String,
    pub supported_event_hooks: Vec<GuardrailEventHooks>,
}

impl MicrosoftPurviewDLPGuardrail {
    #[must_use]
    pub fn new(params: PurviewDlpGuardrailParams, client: reqwest::Client) -> Self {
        let purview_app_name = params
            .purview_app_name
            .unwrap_or_else(|| DEFAULT_APP_NAME.to_owned());
        let user_id_field = params
            .user_id_field
            .unwrap_or_else(|| DEFAULT_USER_ID_FIELD.to_owned());
        let base = PurviewGuardrailBase {
            tenant_id: params.tenant_id,
            client_id: params.client_id,
            client_secret: params.client_secret,
            purview_app_name,
            user_id_field,
            timeout: params.timeout.unwrap_or(DEFAULT_TIMEOUT),
            client,
            graph_api_base: GRAPH_API_BASE.to_owned(),
            login_authority: LOGIN_AUTHORITY.to_owned(),
            clock: SystemTime::now,
            new_id: Uuid::new_v4,
            scope_and_token_cache_lock: Mutex::new(CacheState::default()),
        };
        tracing::info!(
            "Initialized Microsoft Purview DLP Guardrail: {}",
            params.guardrail_name
        );
        Self {
            base,
            guardrail_name: params.guardrail_name,
            supported_event_hooks: hook_methods::get_supported_event_hooks().to_vec(),
        }
    }
}
