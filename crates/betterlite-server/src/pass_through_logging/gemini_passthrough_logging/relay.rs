use betterlite_core::api::media_relay_logging::spend::decimal_json;
use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::passthrough_logging_state::dispatch::PassThroughLoggingKwargs;
use betterlite_logging::passthrough_stream_logging::object::StreamLoggingObject;
use betterlite_models::decisions::Json;
use betterlite_vertex::vertex_relay_metadata::types::{
    CustomLlmProvider, LoggingObject, ModelCallDetails, ResponseLoggingKwargs,
};

// B3638's factory writes through its own logging object; the fields it changes are copied back.
pub(crate) fn relay_logging(logging_obj: &StreamLoggingObject) -> LoggingObject {
    logging_obj
        .read(&mut |state| LoggingObject {
            litellm_call_id: state.litellm_call_id.clone(),
            model: Some(state.model.clone()).filter(|model| !model.is_empty()),
            custom_llm_provider: state
                .custom_llm_provider
                .as_deref()
                .and_then(provider_named),
            model_call_details: ModelCallDetails {
                model: detail_text(state, "model"),
                custom_llm_provider: detail_text(state, "custom_llm_provider")
                    .as_deref()
                    .and_then(provider_named),
                response_cost: None,
            },
        })
        .unwrap_or_default()
}

pub(crate) fn write_relay_logging(logging_obj: &StreamLoggingObject, relay: &LoggingObject) {
    logging_obj.edit(&mut |state| {
        if let Some(model) = &relay.model {
            state.model.clone_from(model);
        }
        if let Some(provider) = relay.custom_llm_provider {
            state.custom_llm_provider = Some(provider.as_str().to_owned());
        }
        if let Some(model) = &relay.model_call_details.model {
            state
                .model_call_details
                .insert("model".to_owned(), Json::Text(model.clone()));
        }
        if let Some(provider) = relay.model_call_details.custom_llm_provider {
            state.model_call_details.insert(
                "custom_llm_provider".to_owned(),
                Json::Text(provider.as_str().to_owned()),
            );
        }
    });
}

// B3638's payloads fill a typed kwargs map; the passthrough seam keeps every caller entry beside them.
pub(crate) fn relay_kwargs(
    requested: &PassThroughLoggingKwargs,
    kwargs: ResponseLoggingKwargs,
) -> PassThroughLoggingKwargs {
    let mut entries = requested.entries.clone();
    if let Some(cost) = kwargs.response_cost {
        entries.insert("response_cost".to_owned(), decimal_json(cost));
    }
    if let Some(model) = kwargs.model {
        entries.insert("model".to_owned(), Json::Text(model));
    }
    if let Some(provider) = kwargs.custom_llm_provider {
        entries.insert(
            "custom_llm_provider".to_owned(),
            Json::Text(provider.as_str().to_owned()),
        );
    }
    PassThroughLoggingKwargs { entries }
}

pub(crate) fn provider_named(name: &str) -> Option<CustomLlmProvider> {
    match name {
        "vertex_ai" => Some(CustomLlmProvider::VertexAi),
        "gemini" => Some(CustomLlmProvider::Gemini),
        _other => None,
    }
}

fn detail_text(state: &LoggingState, key: &str) -> Option<String> {
    match state.model_call_details.get(key) {
        Some(Json::Text(text)) if !text.is_empty() => Some(text.clone()),
        _other => None,
    }
}
