use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

use super::error::StudioInteractionError;

#[skip_serializing_none]
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct InteractionOptions {
    pub tools: Option<Json>,
    pub system_instruction: Option<Json>,
    pub stream: Option<bool>,
    pub store: Option<bool>,
    pub background: Option<bool>,
    pub environment: Option<Json>,
    pub response_modalities: Option<Vec<String>>,
    pub previous_interaction_id: Option<String>,
    pub response_format: Option<Json>,
    #[serde(skip_serializing)]
    pub response_mime_type: Option<String>,
    pub generation_config: Option<IndexMap<String, Json>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum Target {
    Model { model: String },
    Agent { agent: String },
}

#[skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InteractionRequest {
    #[serde(flatten)]
    pub target: Target,
    pub input: Option<Json>,
    #[serde(flatten)]
    pub options: InteractionOptions,
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

// image_config leaves generation_config and joins response_format as an image entry; an emptied config is dropped.
fn move_image_config(options: &mut InteractionOptions) -> Result<(), StudioInteractionError> {
    let Some(config) = options.generation_config.as_mut() else {
        return Ok(());
    };
    let image_config = config.shift_remove("image_config");
    if config.is_empty() {
        options.generation_config = None;
    }
    let image_format = match image_config {
        None | Some(Json::Null) => return Ok(()),
        Some(Json::Object(fields)) => {
            let mut entry = IndexMap::from([("type".to_owned(), Json::Text("image".to_owned()))]);
            entry.extend(fields);
            Json::Object(entry)
        }
        Some(_) => return Err(StudioInteractionError::ImageConfigNotObject),
    };
    options.response_format = Some(match options.response_format.take() {
        None => image_format,
        Some(Json::List(mut entries)) => {
            entries.push(image_format);
            Json::List(entries)
        }
        Some(single) => Json::List(vec![single, image_format]),
    });
    Ok(())
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
    let mut body = options.clone();
    body.response_format = fold_mime_type(
        body.response_format.take(),
        body.response_mime_type.take().as_deref(),
    );
    move_image_config(&mut body)?;
    Ok(InteractionRequest {
        target,
        input: input.filter(|value| !matches!(value, Json::Null)).cloned(),
        options: body,
    })
}
