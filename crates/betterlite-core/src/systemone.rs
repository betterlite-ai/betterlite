use std::time::Duration;

use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_models::decisions::{Extra, Json};
use betterlite_openai::openai_base_config::Headers;
use secrecy::SecretString;

use crate::api::decisions_send::clients::{DecisionsAsyncTransport, DecisionsSyncTransport};
use crate::api::decisions_send::send::DecisionsSender;
use crate::decisions::{DecisionsEntryError, DecisionsPrepare};

mod asystemone;
mod systemone_entrypoint;

pub use asystemone::asystemone;
pub use betterlite_models::decisions::systemone::DecisionsResponse;

use systemone_entrypoint::{prepare_systemone_call, systemone_response};

pub type SystemOneEntryError = DecisionsEntryError;

// litellm.systemone's arguments, with state and the questions map as the caller sent them
pub struct SystemOneArgs {
    pub model: String,
    pub state: Option<Json>,
    pub questions: Option<Json>,
    pub safety_identifier: Option<String>,
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub timeout: Option<Duration>,
    pub custom_llm_provider: Option<String>,
    pub extra_headers: Option<Headers>,
    pub kwargs: Extra,
}

pub struct SystemOneDeps<'deps, Clients> {
    pub clients: &'deps mut Clients,
    pub env: &'deps PrePostEnv<'deps>,
    pub prepare: DecisionsPrepare<'deps>,
}

// litellm.systemone: _prepare_systemone_call, B3514's blocking send, then _systemone_response
pub fn systemone<Clients>(
    deps: SystemOneDeps<'_, Clients>,
    args: SystemOneArgs,
) -> Result<DecisionsResponse, SystemOneEntryError>
where
    Clients: DecisionsSyncTransport + DecisionsAsyncTransport,
{
    let call = prepare_systemone_call(deps.prepare, args)?;
    let mut sender = DecisionsSender::new(deps.clients, deps.env);
    let output = sender.send(&call)?;
    systemone_response(output)
}
