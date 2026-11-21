use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use betterlite_auth::auth_user_lookup::error::UserLookupError;
use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::guardrail_params::opaque::OpaqueParams;
use betterlite_guardrails::guardrail_registry::rows::GuardrailRows;
use betterlite_guardrails::guardrail_startup::runtime::GuardrailRuntime;
use betterlite_pyhost::json_values::JsonObject;

pub struct ReadState<R: GuardrailRows> {
    pub runtime: Arc<GuardrailRuntime<R>>,
    // B8610's Shared answers a database error from the rows value without a database, as LiteLLM does with prisma None
    pub database: bool,
    pub config: Arc<dyn ConfigGuardrails>,
    pub teams: Arc<dyn UserTeams>,
    pub environment: Arc<dyn EnvironmentLookup>,
}

// the loaded config file's guardrails list as written, which the typed section drops unknown params from
#[derive(Clone, Debug, PartialEq)]
pub struct ConfigGuardrail {
    pub guardrail_id: Option<String>,
    pub guardrail_name: String,
    pub litellm_params: Option<JsonObject>,
    pub guardrail_info: Option<OpaqueParams>,
}

// proxy_config.config.get("guardrails"), filled once at mount from the loaded config document
pub trait ConfigGuardrails: Send + Sync {
    fn guardrails(&self) -> Vec<ConfigGuardrail>;
}

pub type TeamIdsFuture<'read> =
    Pin<Box<dyn Future<Output = Result<Vec<String>, UserLookupError>> + Send + 'read>>;

// _get_user_team_ids reads the user row through B736's get_user_object, which a port keeps out of the route
pub trait UserTeams: Send + Sync {
    fn team_ids(&self, user_id: &str) -> TeamIdsFuture<'_>;
}
