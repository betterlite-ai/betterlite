use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_state::updates::{CallUpdate, update_environment_variables};
use betterlite_models::decisions::Json;
use betterlite_openai::openai_base_config::Object;
use betterlite_openai::speech::logging::{SpeechLogRecord, SpeechLogger};
use indexmap::IndexMap;
use serde_json::Value;

use super::call::SpeechCall;
use super::error::SpeechCallError;
use super::values;

// the logging object speech() receives: its state for the call record, and the pre-call hook B514 may write
pub trait SpeechCallLogging {
    fn state(&mut self) -> &mut LoggingState;

    fn pre_call(&mut self, record: SpeechLogRecord<'_>) -> Result<(), PrePostError>;
}

// a caller that holds only the state logs the pre-call record nowhere
impl SpeechCallLogging for LoggingState {
    fn state(&mut self) -> &mut LoggingState {
        self
    }

    fn pre_call(&mut self, _record: SpeechLogRecord<'_>) -> Result<(), PrePostError> {
        Ok(())
    }
}

// forwards audio_speech's hook to the caller's logging object
pub(crate) struct SpeechLoggerAdapter<'logging> {
    pub(crate) logging: &'logging mut dyn SpeechCallLogging,
}

impl SpeechLogger for SpeechLoggerAdapter<'_> {
    fn pre_call(&mut self, record: SpeechLogRecord<'_>) -> Result<(), PrePostError> {
        self.logging.pre_call(record)
    }
}

// litellm's logging_obj.update_environment_variables over speech's model, options and kwargs
pub(crate) fn record_call(
    logging: &mut dyn SpeechCallLogging,
    call: &SpeechCall,
    model: &str,
    provider: &str,
    optional_params: &Object,
    kwargs: &Object,
) -> Result<(), SpeechCallError> {
    let params = values::json_object(&log_params(call, kwargs))?;
    let options = values::json_object(optional_params)?;
    let user = values::text(kwargs, "user").map(str::to_owned);
    let additional = IndexMap::from([(
        String::from("custom_llm_provider"),
        Json::Text(provider.to_owned()),
    )]);
    update_environment_variables(
        logging.state(),
        &params,
        CallUpdate {
            model: Some(model.to_owned()),
            user,
            optional_params: options,
            additional_params: additional,
        },
    )
    .map_err(|source| SpeechCallError::Internal {
        message: source.to_string(),
    })
}

// the dict speech passes: its five fixed slots with the remaining kwargs laid over them
fn log_params(call: &SpeechCall, kwargs: &Object) -> Object {
    let mut params = Object::new();
    for name in ["litellm_call_id", "proxy_server_request", "model_info"] {
        if let Some(value) = kwargs.get(name) {
            params.insert(name.to_owned(), value.clone());
        }
    }
    params.insert(
        String::from("metadata"),
        call.metadata.clone().map_or(Value::Null, Value::Object),
    );
    params.insert(String::from("preset_cache_key"), Value::Null);
    params.insert(
        String::from("stream_response"),
        Value::Object(Object::new()),
    );
    for (name, value) in kwargs {
        params.insert(name.clone(), value.clone());
    }
    params
}
