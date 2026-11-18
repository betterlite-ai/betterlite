use super::error::RagflowError;

const PROVIDER: &str = "ragflow";
const CHAT: &str = "chat";
const AGENT: &str = "agent";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndpointType {
    Chat,
    Agent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedModel {
    pub endpoint: EndpointType,
    pub entity_id: String,
    pub model: String,
}

mod tests;

// LiteLLM's _parse_ragflow_model: split on "/", require four parts, then keep the rest as the model name
pub fn parse_model(model: &str) -> Result<ParsedModel, RagflowError> {
    let mut parts = model.split('/');
    let first = parts.next();
    let second = parts.next();
    let third = parts.next();
    let model_name: Vec<&str> = parts.collect();
    let (Some(provider), Some(endpoint), Some(entity_id)) = (first, second, third) else {
        return Err(RagflowError::ModelTooShort {
            model: model.to_owned(),
        });
    };
    // Python checks the part count before the prefix, so a short model with another prefix is still too short
    if model_name.is_empty() {
        return Err(RagflowError::ModelTooShort {
            model: model.to_owned(),
        });
    }
    if provider != PROVIDER {
        return Err(RagflowError::ModelWrongPrefix {
            model: model.to_owned(),
        });
    }
    let endpoint = match endpoint {
        CHAT => EndpointType::Chat,
        AGENT => EndpointType::Agent,
        other => {
            return Err(RagflowError::EndpointType {
                endpoint_type: other.to_owned(),
            });
        }
    };
    Ok(ParsedModel {
        endpoint,
        entity_id: entity_id.to_owned(),
        model: model_name.join("/"),
    })
}
