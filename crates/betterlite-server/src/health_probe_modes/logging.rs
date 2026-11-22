use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_logging::logging_state::updates::{CallUpdate, update_from_kwargs as apply_update};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use std::sync::{Arc, Mutex, PoisonError};
// the arguments LiteLLM hands Logging.update_from_kwargs, with every value typed as the logging state holds it
pub struct ProbeLoggingUpdate {
    pub kwargs: IndexMap<String, Json>,
    pub model: Option<String>,
    pub user: Option<String>,
    pub optional_params: IndexMap<String, Json>,
    pub litellm_params: Option<IndexMap<String, Json>>,
}
// the logging object a health check builds, narrowed to the one update this module makes on it
pub trait HealthProbeLogging: Send + Sync {
    fn update_from_kwargs(&self, update: ProbeLoggingUpdate) -> Result<(), LoggingStateError>;
    // the state itself, for a backend call that logs through the same object
    fn logging_state(&self) -> Option<Arc<Mutex<LoggingState>>>;
}
pub struct ProbeLogging {
    state: Arc<Mutex<LoggingState>>,
}
impl From<LoggingState> for ProbeLogging {
    fn from(state: LoggingState) -> Self {
        Self {
            state: Arc::new(Mutex::new(state)),
        }
    }
}
impl HealthProbeLogging for ProbeLogging {
    fn update_from_kwargs(&self, update: ProbeLoggingUpdate) -> Result<(), LoggingStateError> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let call_update = CallUpdate {
            model: update.model,
            user: update.user,
            optional_params: update.optional_params,
            additional_params: IndexMap::new(),
        };
        apply_update(
            &mut state,
            &update.kwargs,
            update.litellm_params.as_ref(),
            call_update,
        )
    }
    fn logging_state(&self) -> Option<Arc<Mutex<LoggingState>>> {
        let state = Arc::clone(&self.state);
        Some(state)
    }
}
