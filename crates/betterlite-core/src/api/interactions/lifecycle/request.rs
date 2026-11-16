use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_config::env_integer::timeout::RequestTimeout;
use betterlite_interactions::polling::host::FetchInteraction;
use betterlite_interactions::polling::registry::BackgroundPollRegistry;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

// the caller's side of delete and cancel, the parameters LiteLLM's SDK methods take
pub struct InteractionLifecycleRequest<'request> {
    pub interaction_id: &'request str,
    pub extra_headers: BTreeMap<String, String>,
    pub timeout: Option<Duration>,
    pub custom_llm_provider: Option<&'request str>,
    // B198's REQUEST_TIMEOUT, the deadline LiteLLM falls back to when the caller names none
    pub request_timeout: RequestTimeout,
    pub kwargs: &'request IndexMap<String, Json>,
}

// adelete settles the tracked background interaction before the delete removes it
pub struct BackgroundSettlement<'settle, Host, FetchError> {
    pub registry: &'settle BackgroundPollRegistry<Host>,
    pub fetch: FetchInteraction<Host, FetchError>,
}
