use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use super::error::StudioInteractionError;

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct InteractionOptions {
    pub tools: Option<Json>,
    pub system_instruction: Option<Json>,
    pub generation_config: Option<IndexMap<String, Json>>,
    pub stream: Option<bool>,
    pub store: Option<bool>,
    pub background: Option<bool>,
    pub environment: Option<Json>,
    pub response_modalities: Option<Vec<String>>,
    pub response_format: Option<Json>,
    pub response_mime_type: Option<String>,
    pub previous_interaction_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum Target {
    Model { model: String },
    Agent { agent: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InteractionRequest {
    #[serde(flatten)]
    pub target: Target,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<Json>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Json>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_instruction: Option<Json>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub store: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub background: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub environment: Option<Json>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_modalities: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_interaction_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_format: Option<Json>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generation_config: Option<IndexMap<String, Json>>,
}

// A name that is only the "gemini/" prefix keeps the prefix, as `get_base_model(model) or model` does.
fn base_model(model: &str) -> String {
    let stripped = model.replace("gemini/", "");
    if stripped.is_empty() {
        model.to_owned()
    } else {
        stripped
    }
}

// A list or a format that already names a mime_type is left alone.
fn fold_mime_type(response_format: Option<Json>, mime_type: Option<&str>) -> Option<Json> {
    let Some(mime) = mime_type.filter(|mime| !mime.is_empty()) else {
        return response_format;
    };
    match &response_format {
        Some(Json::List(_)) => return response_format,
        Some(Json::Object(fields)) if fields.contains_key("mime_type") => return response_format,
        _ => {}
    }
    let mut wrapped = IndexMap::from([
        ("type".to_owned(), Json::Text("text".to_owned())),
        ("mime_type".to_owned(), Json::Text(mime.to_owned())),
    ]);
    if let Some(schema) = response_format {
        wrapped.insert("schema".to_owned(), schema);
    }
    Some(Json::Object(wrapped))
}

struct ImageSplit {
    generation_config: Option<IndexMap<String, Json>>,
    response_format: Option<Json>,
}

// image_config leaves generation_config and joins response_format as an image entry; an emptied config is dropped.
fn move_image_config(
    generation_config: Option<&IndexMap<String, Json>>,
    response_format: Option<Json>,
) -> Result<ImageSplit, StudioInteractionError> {
    let Some(source) = generation_config else {
        return Ok(ImageSplit {
            generation_config: None,
            response_format,
        });
    };
    let mut config = source.clone();
    let image_config = config.shift_remove("image_config");
    let kept = Some(config).filter(|fields| !fields.is_empty());
    let image_format = match image_config {
        None | Some(Json::Null) => {
            return Ok(ImageSplit {
                generation_config: kept,
                response_format,
            });
        }
        Some(Json::Object(fields)) => {
            let mut entry = IndexMap::from([("type".to_owned(), Json::Text("image".to_owned()))]);
            entry.extend(fields);
            Json::Object(entry)
        }
        Some(_) => return Err(StudioInteractionError::ImageConfigNotObject),
    };
    let formats = match response_format {
        None => image_format,
        Some(Json::List(mut entries)) => {
            entries.push(image_format);
            Json::List(entries)
        }
        Some(single) => Json::List(vec![single, image_format]),
    };
    Ok(ImageSplit {
        generation_config: kept,
        response_format: Some(formats),
    })
}

pub fn transform_request(
    model: Option<&str>,
    agent: Option<&str>,
    input: Option<&Json>,
    options: &InteractionOptions,
) -> Result<InteractionRequest, StudioInteractionError> {
    let target = match (model.filter(|name| !name.is_empty()), agent) {
        (Some(name), _) => Target::Model {
            model: base_model(name),
        },
        (None, Some(name)) if !name.is_empty() => Target::Agent {
            agent: name.to_owned(),
        },
        (None, _) => return Err(StudioInteractionError::MissingTarget),
    };
    let folded = fold_mime_type(
        options.response_format.clone(),
        options.response_mime_type.as_deref(),
    );
    let split = move_image_config(options.generation_config.as_ref(), folded)?;
    Ok(InteractionRequest {
        target,
        input: input.filter(|value| !matches!(value, Json::Null)).cloned(),
        tools: options.tools.clone(),
        system_instruction: options.system_instruction.clone(),
        stream: options.stream,
        store: options.store,
        background: options.background,
        environment: options.environment.clone(),
        response_modalities: options.response_modalities.clone(),
        previous_interaction_id: options.previous_interaction_id.clone(),
        response_format: split.response_format,
        generation_config: split.generation_config,
    })
}
