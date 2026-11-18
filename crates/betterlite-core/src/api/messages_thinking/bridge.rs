use std::sync::Arc;

use betterlite_logging::passthrough_stream_logging::object::StreamLoggingObject;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

// The bridged call's kwargs; litellm_logging_obj holds B247's object, so a lookalike mapping has no arm.
#[derive(Clone, Default)]
pub struct BridgeKwargs {
    pub litellm_logging_obj: Option<Arc<StreamLoggingObject>>,
    pub request_body: IndexMap<String, Json>,
}

// local_model_name: the provider prefix comes off once, and only when the caller named a provider.
#[must_use]
pub fn local_model_name<'model>(
    model: &'model str,
    custom_llm_provider: Option<&str>,
) -> &'model str {
    match custom_llm_provider {
        Some(provider) => model.strip_prefix(&format!("{provider}/")).unwrap_or(model),
        None => model,
    }
}
