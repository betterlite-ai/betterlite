use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_logging::logging_state::updates::CallUpdate;
use betterlite_logging::logging_state::updates::update_from_kwargs;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::error::SandboxLifecycleError;
use super::request::SandboxCall;

pub struct SandboxLogUpdate<'call> {
    pub kwargs: &'call IndexMap<String, Json>,
    pub model: String,
    pub litellm_params: IndexMap<String, Json>,
    pub custom_llm_provider: String,
}

pub trait SandboxLogging: Send {
    fn update(&mut self, update: SandboxLogUpdate<'_>) -> Result<(), LoggingStateError>;
}

impl SandboxLogging for LoggingState {
    fn update(&mut self, update: SandboxLogUpdate<'_>) -> Result<(), LoggingStateError> {
        let mut additional_params = IndexMap::new();
        additional_params.insert(
            "custom_llm_provider".to_owned(),
            Json::Text(update.custom_llm_provider),
        );
        update_from_kwargs(
            self,
            update.kwargs,
            Some(&update.litellm_params),
            CallUpdate {
                model: Some(update.model),
                user: None,
                optional_params: IndexMap::new(),
                additional_params,
            },
        )
    }
}

pub(super) fn update_logging(
    call: &mut SandboxCall<'_>,
    operation: &str,
) -> Result<(), SandboxLifecycleError> {
    let Some(logging) = call.logging.as_mut() else {
        return Ok(());
    };
    let mut litellm_params = IndexMap::new();
    litellm_params.insert(
        "litellm_call_id".to_owned(),
        call.kwargs
            .get("litellm_call_id")
            .cloned()
            .unwrap_or(Json::Null),
    );
    logging.update(SandboxLogUpdate {
        kwargs: call.kwargs,
        model: format!("{}/{operation}", call.provider),
        litellm_params,
        custom_llm_provider: call.provider.to_owned(),
    })?;
    Ok(())
}
