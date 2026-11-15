use betterlite_models::decisions::{Extra, Json as DecisionJson};
use betterlite_models::provider_model_sets::{ProviderModelSet, ProviderModelSets};
use betterlite_openai::chat::{ChatMessage, MessageContent};
use betterlite_prompts::arguments::json::{Json, Object};

use super::LegacyDeps;
use super::LegacyRequest;
use super::error::LegacyVertexError;
use crate::legacy_prediction::records::source_string;

pub(crate) struct LegacyPlan {
    pub(crate) model: String,
    pub(crate) prompt: String,
    pub(crate) route: LegacyRoute,
    pub(crate) stream: bool,
    pub(crate) acompletion: bool,
}

// One route per selection branch of the source; the vision route has no invocation of its own
pub(crate) enum LegacyRoute {
    Vision,
    Chat {
        options: Extra,
    },
    Text {
        options: Extra,
        fake_stream: bool,
    },
    Custom {
        instances: Vec<DecisionJson>,
        options: Extra,
    },
    Private {
        instances: Vec<DecisionJson>,
        options: Extra,
    },
}

pub(crate) fn plan(
    request: LegacyRequest<'_>,
    deps: &LegacyDeps<'_, '_>,
) -> Result<LegacyPlan, LegacyVertexError> {
    let LegacyRequest {
        model,
        messages,
        mut options,
        acompletion,
    } = request;
    // compatibility.md: an absent or null option takes its default, and LiteLLM's optional params never hold a None
    options.retain(|_name, value| !matches!(value, Json::Null));
    deps.defaults.merge_into(&mut options)?;
    validate_safety(options.shift_remove("safety_settings"))?;
    let stream = matches!(options.get("stream"), Some(Json::Bool(true)));
    let prompt = prompt_of(messages);
    let (model, route) = route_of(model, &mut options, &prompt, deps)?;
    Ok(LegacyPlan {
        model,
        prompt,
        route,
        stream,
        acompletion,
    })
}

// The source checks the model groups in this order, and the private literal before the custom fallback
fn route_of(
    model: &str,
    options: &mut Object,
    prompt: &str,
    deps: &LegacyDeps<'_, '_>,
) -> Result<(String, LegacyRoute), LegacyVertexError> {
    if member(deps.sets, ProviderModelSet::VertexLanguage, model)
        || member(deps.sets, ProviderModelSet::VertexVision, model)
    {
        return Ok((model.to_owned(), LegacyRoute::Vision));
    }
    if member(deps.sets, ProviderModelSet::VertexChat, model) {
        let route = LegacyRoute::Chat {
            options: extra(options),
        };
        return Ok((model.to_owned(), route));
    }
    if member(deps.sets, ProviderModelSet::VertexText, model) {
        let route = LegacyRoute::Text {
            options: extra(options),
            fake_stream: false,
        };
        return Ok((model.to_owned(), route));
    }
    if member(deps.sets, ProviderModelSet::VertexCodeText, model) {
        let route = LegacyRoute::Text {
            options: extra(options),
            fake_stream: true,
        };
        return Ok((model.to_owned(), route));
    }
    if member(deps.sets, ProviderModelSet::VertexCodeChat, model) {
        let route = LegacyRoute::Chat {
            options: extra(options),
        };
        return Ok((model.to_owned(), route));
    }
    if model == "private" {
        return private_route(options, prompt);
    }
    custom_route(model, options, prompt, deps)
}

fn private_route(
    options: &mut Object,
    prompt: &str,
) -> Result<(String, LegacyRoute), LegacyVertexError> {
    let Some(value) = options.shift_remove("model_id") else {
        return Err(LegacyVertexError::MissingModelId);
    };
    let instances = instance(options, prompt);
    let route = LegacyRoute::Private {
        instances,
        options: extra(options),
    };
    Ok((source_string(&value), route))
}

fn custom_route(
    model: &str,
    options: &mut Object,
    prompt: &str,
    deps: &LegacyDeps<'_, '_>,
) -> Result<(String, LegacyRoute), LegacyVertexError> {
    if deps.project.is_none() || deps.location.is_none() {
        return Err(LegacyVertexError::CustomEndpointContext);
    }
    let instances = instance(options, prompt);
    let route = LegacyRoute::Custom {
        instances,
        options: extra(options),
    };
    Ok((model.to_owned(), route))
}

// The source copies the options once, after safety_settings and (for private) model_id are gone
fn instance(options: &Object, prompt: &str) -> Vec<DecisionJson> {
    let mut fields = options.clone();
    fields.insert(String::from("prompt"), Json::Text(prompt.to_owned()));
    vec![DecisionJson::Object(extra(&fields))]
}

// LiteLLM checks the first item, then builds a protobuf setting from every item, which fails on a non-mapping
fn validate_safety(safety: Option<Json>) -> Result<(), LegacyVertexError> {
    let Some(safety) = safety else {
        return Ok(());
    };
    let Json::List(items) = safety else {
        return Err(LegacyVertexError::SafetyNotList);
    };
    if items.iter().any(|item| !matches!(item, Json::Object(_))) {
        return Err(LegacyVertexError::SafetyItemNotMapping);
    }
    Ok(())
}

// LiteLLM's " ".join over the message contents the isinstance(content, str) filter keeps
fn prompt_of(messages: &[ChatMessage]) -> String {
    messages
        .iter()
        .filter_map(|message| match &message.content {
            Some(MessageContent::Text(text)) => Some(text.as_str()),
            Some(MessageContent::Parts(_parts)) | None => None,
        })
        .collect::<Vec<&str>>()
        .join(" ")
}

fn member(sets: &ProviderModelSets, set: ProviderModelSet, model: &str) -> bool {
    sets.sets
        .get(&set)
        .is_some_and(|models| models.contains(model))
}

fn extra(options: &Object) -> Extra {
    options
        .iter()
        .map(|(name, value)| (name.clone(), decision(value)))
        .collect()
}

fn decision(value: &Json) -> DecisionJson {
    match value {
        Json::Null => DecisionJson::Null,
        Json::Bool(found) => DecisionJson::Bool(*found),
        Json::Number(found) => DecisionJson::Number(found.clone()),
        Json::Text(found) => DecisionJson::Text(found.clone()),
        Json::List(items) => DecisionJson::List(items.iter().map(decision).collect()),
        Json::Object(fields) => DecisionJson::Object(extra(fields)),
    }
}

pub(crate) fn endpoint_path(project: &str, location: &str, model: &str) -> String {
    let mut endpoint = format!("projects/{project}/locations/{location}/endpoints/");
    endpoint.push_str(model);
    endpoint
}
