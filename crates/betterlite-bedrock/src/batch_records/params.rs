use betterlite_context::owned_parameter_names::is_litellm_owned_kwarg;
use betterlite_context::provider_message_primitives::{
    ProviderMessage, Record, developer_to_system,
};
use betterlite_models::decisions::Json;
use betterlite_models::resolution::ResolutionContext;
use indexmap::IndexMap;

use super::error::BatchRecordsError;
use super::families::{BatchFamilies, Params};
use crate::aws_endpoints::provider::BedrockInvokeProvider;
use crate::s3_access::models::invoke_route_model;

const ALLOWED_OPENAI_PARAMS: &str = "allowed_openai_params";
const CONTENT_FIELD: &str = "content";
const JSON_MODE_FIELD: &str = "json_mode";
const MESSAGES_FIELD: &str = "messages";
const MODEL_FIELD: &str = "model";
const ROLE_FIELD: &str = "role";
const SYSTEM_ROLE: &str = "system";

// _map_openai_to_bedrock_params: the chat body becomes the InvokeModel body each provider family expects.
pub async fn map_openai_to_bedrock_params(
    body: &Json,
    model: &str,
    provider: Option<BedrockInvokeProvider>,
    families: &dyn BatchFamilies,
    resolution: &ResolutionContext<'_>,
) -> Result<Json, BatchRecordsError> {
    let Json::Object(fields) = body else {
        return Err(BatchRecordsError::RecordShape);
    };
    let messages = projected_messages(fields.get(MESSAGES_FIELD))?;
    let optional = optional_params(fields);
    match provider {
        Some(BedrockInvokeProvider::Anthropic) => {
            anthropic_body(model, &messages, &optional, families, resolution).await
        }
        Some(BedrockInvokeProvider::Nova) => families
            .nova_body(model, &messages, &optional)
            .await
            .map_err(|source| BatchRecordsError::Family {
                family: "nova",
                source,
            }),
        Some(_) | None => Ok(passthrough_body(messages, optional)),
    }
}

async fn anthropic_body(
    model: &str,
    messages: &[Json],
    optional: &Params,
    families: &dyn BatchFamilies,
    resolution: &ResolutionContext<'_>,
) -> Result<Json, BatchRecordsError> {
    let filtered = without_litellm_kwargs(optional);
    let mapped = families
        .anthropic_params(&invoke_route_model(model, resolution), &filtered)
        .await
        .map_err(|source| BatchRecordsError::Family {
            family: "anthropic",
            source,
        })?;
    let mut stripped = mapped;
    // _MAPPED_PARAMS_THE_REQUEST_HANDLER_STRIPS: the handler strips json_mode after the mapper adds it.
    stripped.shift_remove(JSON_MODE_FIELD);
    families
        .anthropic_body(model, messages, &stripped)
        .await
        .map_err(|source| BatchRecordsError::Family {
            family: "anthropic",
            source,
        })
}

// `{k: v for k, v in body.items() if k not in ["model", "messages"]}`, in body order.
fn optional_params(fields: &IndexMap<String, Json>) -> Params {
    fields
        .iter()
        .filter(|(name, _value)| *name != MODEL_FIELD && *name != MESSAGES_FIELD)
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}

// `{"messages": messages, **optional_params}`: the messages are inserted first.
fn passthrough_body(messages: Vec<Json>, optional: Params) -> Json {
    let mut body = Params::new();
    body.insert(String::from(MESSAGES_FIELD), Json::List(messages));
    body.extend(optional);
    Json::Object(body)
}

// A litellm-owned kwarg never reaches the mapper except for the ones the mapper takes itself.
fn without_litellm_kwargs(optional: &Params) -> Params {
    optional
        .iter()
        .filter(|(name, _value)| {
            !is_litellm_owned_kwarg(name.as_str()) || name.as_str() == ALLOWED_OPENAI_PARAMS
        })
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}

// `map_developer_role_to_system_role(body.get("messages", []))`: absent is empty, anything but a list is refused.
fn projected_messages(value: Option<&Json>) -> Result<Vec<Json>, BatchRecordsError> {
    let entries = match value {
        Some(Json::List(items)) => items.clone(),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_)) => {
            return Err(BatchRecordsError::Field {
                field: MESSAGES_FIELD,
            });
        }
        None => Vec::new(),
    };
    let mut records = Vec::with_capacity(entries.len());
    let mut objects = Vec::with_capacity(entries.len());
    for entry in entries {
        let Json::Object(fields) = entry else {
            return Err(BatchRecordsError::Field {
                field: MESSAGES_FIELD,
            });
        };
        records.push(message_record(&fields));
        objects.push(fields);
    }
    let projected = developer_to_system(records)?;
    let mut bodies = Vec::with_capacity(projected.len());
    for (message, fields) in projected.into_iter().zip(objects) {
        bodies.push(projected_message(message, fields));
    }
    Ok(bodies)
}

// developer_to_system reads the role and drops it into a new dict, so every field travels as it was parsed.
fn message_record(fields: &IndexMap<String, Json>) -> Record<MessageField> {
    fields
        .iter()
        .map(|(name, value)| (name.clone(), Some(MessageField(value.clone()))))
        .collect()
}

// Python writes a developer message as {"role": "system", "content": ...} and keeps every other dict as it was.
fn projected_message(
    message: ProviderMessage<MessageField>,
    fields: IndexMap<String, Json>,
) -> Json {
    match message {
        ProviderMessage::System { content, .. } => {
            let mut system = IndexMap::new();
            system.insert(
                String::from(ROLE_FIELD),
                Json::Text(String::from(SYSTEM_ROLE)),
            );
            system.insert(
                String::from(CONTENT_FIELD),
                content.map_or(Json::Null, |field| field.0),
            );
            Json::Object(system)
        }
        ProviderMessage::Unchanged(_) => Json::Object(fields),
    }
}

// developer_to_system compares a message's role against `str`, so the role field is a newtype over `Json`.
#[derive(Debug)]
struct MessageField(Json);

impl PartialEq<str> for MessageField {
    fn eq(&self, other: &str) -> bool {
        matches!(&self.0, Json::Text(text) if text == other)
    }
}
