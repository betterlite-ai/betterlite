use betterlite_context::media_config_selection::{
    audio_transcription, image_edit, image_generation, text_to_speech,
};
use betterlite_models::identity::provider_id::ProviderId;

use betterlite_context::resource_config_selection::{
    messages, passthrough, realtime, responses, search_harness, skills_evals, vector_stores,
};
use serde_json::{Value, json};

use super::{BatchInput, Call, catalog, facts_of, invalid, model_of, provider_of};
use crate::helpers::HelperError;

pub(crate) type CallResult = Result<Value, Box<HelperError>>;

fn arguments_of<T>(tag: &T) -> CallResult
where
    T: serde::Serialize,
{
    let serialized = serde_json::to_value(tag).map_err(|error| invalid(&error.to_string()))?;
    match serialized {
        Value::Object(fields) => fields
            .into_iter()
            .next()
            .map(|(_, arguments)| arguments)
            .ok_or_else(|| invalid("a tag serialized to an empty object")),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) | Value::Array(_) => {
            Ok(json!([]))
        }
    }
}

fn unit_report<T>(selected: Option<T>, suffix: &str) -> Value
where
    T: std::fmt::Debug,
{
    selected.map_or_else(
        || Value::Null,
        |tag| json!({"class": format!("{tag:?}{suffix}"), "arguments": []}),
    )
}

pub(crate) fn transcription(input: &BatchInput, call: &Call) -> CallResult {
    let live = catalog(&input.model_cost)?;
    let bundled = catalog(&input.bundled_model_cost)?;
    let selected = audio_transcription::select_transcription_config(
        model_of(call)?,
        provider_of(call)?,
        &live,
        &bundled,
    );
    Ok(unit_report(selected, "AudioTranscriptionConfig"))
}

pub(crate) fn text_to_speech(_input: &BatchInput, call: &Call) -> CallResult {
    Ok(unit_report(
        text_to_speech::select_text_to_speech(model_of(call)?, provider_of(call)?, call.is_lyria),
        "TextToSpeechConfig",
    ))
}

pub(crate) fn responses(_input: &BatchInput, call: &Call) -> CallResult {
    let facts = facts_of(&call.facts);
    match responses::select_responses_config(call.provider, call.model.as_deref(), &facts) {
        Some(tag) => Ok(json!({"class": tag.class_name(), "arguments": arguments_of(&tag)?})),
        None => Ok(Value::Null),
    }
}

pub(crate) fn vector_stores(_input: &BatchInput, call: &Call) -> CallResult {
    let provider = provider_of(call)?;
    let selected = vector_stores::select_vector_stores_config(provider, call.api_type.as_deref());
    let pg = provider == ProviderId::PgVector;
    let suffix = if pg {
        "StoreConfig"
    } else {
        "VectorStoreConfig"
    };
    Ok(unit_report(selected, suffix))
}

fn search_class(tag: search_harness::SearchConfigTag) -> String {
    if tag == search_harness::SearchConfigTag::SearchAPI {
        return "SearchAPIConfig".to_owned();
    }
    format!("{tag:?}SearchConfig")
}

pub(crate) fn search(_input: &BatchInput, call: &Call) -> CallResult {
    let provider = call
        .search_provider
        .ok_or_else(|| invalid("search missing"))?;
    let tag = search_harness::select_search_config(provider);
    Ok(json!({"class": search_class(tag), "arguments": []}))
}

pub(crate) fn harness(_input: &BatchInput, call: &Call) -> CallResult {
    let harness = call.harness.ok_or_else(|| invalid("harness is missing"))?;
    let tag = search_harness::select_harness_config(harness);
    Ok(json!({"class": format!("{tag:?}HarnessConfig"), "arguments": []}))
}

pub(crate) fn image_generation(_input: &BatchInput, call: &Call) -> CallResult {
    let selected = image_generation::select_image_generation(model_of(call)?, provider_of(call)?);
    let Some(tag) = selected else {
        return Ok(Value::Null);
    };
    Ok(json!({"class": tag.class_name(), "arguments": arguments_of(&tag)?}))
}

pub(crate) fn image_edit(_input: &BatchInput, call: &Call) -> CallResult {
    let selected = image_edit::select_image_edit(model_of(call)?, provider_of(call)?);
    let Some(tag) = selected else {
        return Ok(Value::Null);
    };
    Ok(json!({"class": tag.class_name(), "arguments": arguments_of(&tag)?}))
}

pub(crate) fn messages(_input: &BatchInput, call: &Call) -> CallResult {
    let selected = messages::select_messages_config(model_of(call)?, provider_of(call)?);
    let Some(tag) = selected else {
        return Ok(Value::Null);
    };
    Ok(json!({"class": tag.class_name(), "arguments": arguments_of(&tag)?}))
}

pub(crate) fn passthrough(_input: &BatchInput, call: &Call) -> CallResult {
    Ok(unit_report(
        passthrough::select_passthrough_config(model_of(call)?, provider_of(call)?),
        "PassthroughConfig",
    ))
}

pub(crate) fn batches(_input: &BatchInput, call: &Call) -> CallResult {
    Ok(unit_report(
        passthrough::select_batches_config(model_of(call)?, provider_of(call)?),
        "BatchesConfig",
    ))
}

pub(crate) fn vector_store(_input: &BatchInput, call: &Call) -> CallResult {
    Ok(unit_report(
        vector_stores::select_vector_store_config(provider_of(call)?),
        "VectorStore",
    ))
}

pub(crate) fn vector_store_files(_input: &BatchInput, call: &Call) -> CallResult {
    let provider = provider_of(call)?;
    let selected = vector_stores::select_store_files_config(provider);
    Ok(unit_report(selected, "VectorStoreFilesConfig"))
}

pub(crate) fn realtime(_input: &BatchInput, call: &Call) -> CallResult {
    Ok(unit_report(
        realtime::select_realtime_config(model_of(call)?, provider_of(call)?),
        "RealtimeConfig",
    ))
}

pub(crate) fn realtime_http(_input: &BatchInput, call: &Call) -> CallResult {
    Ok(unit_report(
        realtime::select_realtime_http_config(model_of(call)?, provider_of(call)?),
        "RealtimeHTTPConfig",
    ))
}

pub(crate) fn skills(_input: &BatchInput, call: &Call) -> CallResult {
    let provider = provider_of(call)?;
    let selected = skills_evals::select_skills_config(provider);
    Ok(unit_report(selected, "SkillsConfig"))
}

pub(crate) fn evals(_input: &BatchInput, call: &Call) -> CallResult {
    let provider = provider_of(call)?;
    let selected = skills_evals::select_evals_config(provider);
    Ok(unit_report(selected, "EvalsConfig"))
}
