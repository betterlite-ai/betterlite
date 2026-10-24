use std::collections::BTreeMap;

use serde::Serialize;
use uuid::Uuid;

use crate::error::ContextError;

mod tests;

pub const RESPONSE_FORMAT_TOOL: &str = "json_tool_call";

// None is a JSON null, so a key set to null stays apart from an absent key
pub type Record<Field> = BTreeMap<String, Option<Field>>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Schema<Field> {
    Falsy,
    Truthy(Field),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NestedSchema<Field> {
    Object(Option<Schema<Field>>),
    NotObject,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResponseFormat<Field> {
    pub value: Field,
    pub response_schema: Option<Schema<Field>>,
    pub json_schema: Option<NestedSchema<Field>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormatSupport {
    Native,
    ToolCall,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChoicePolicy {
    Enforce,
    KeepExisting,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FunctionKind {
    Function,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FunctionTool<Field> {
    #[serde(rename = "type")]
    pub kind: FunctionKind,
    pub function: ToolFunction<Field>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ToolFunction<Field> {
    pub name: &'static str,
    pub parameters: Field,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FunctionChoice {
    #[serde(rename = "type")]
    pub kind: FunctionKind,
    pub function: ChoiceFunction,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChoiceFunction {
    pub name: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum ToolParam<Field> {
    Supplied(Field),
    ResponseFormat(FunctionTool<Field>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum ToolChoice<Field> {
    Supplied(Field),
    ResponseFormat(FunctionChoice),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FormatParams<Field> {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<ToolParam<Field>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<ToolChoice<Field>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub json_mode: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_format: Option<Field>,
}

pub fn add_response_format<Field>(
    params: &mut FormatParams<Field>,
    format: ResponseFormat<Field>,
    support: FormatSupport,
    policy: ChoicePolicy,
) -> Result<(), ContextError> {
    let schema = match (format.response_schema, format.json_schema) {
        (Some(schema), _) | (None, Some(NestedSchema::Object(Some(schema)))) => schema,
        (None, Some(NestedSchema::Object(None))) => return Err(ContextError::MissingSchema),
        (None, Some(NestedSchema::NotObject)) => return Err(ContextError::SchemaNotObject),
        (None, None) => Schema::Falsy,
    };
    match (schema, support) {
        (Schema::Truthy(parameters), FormatSupport::ToolCall) => {
            let function = ToolFunction {
                name: RESPONSE_FORMAT_TOOL,
                parameters,
            };
            let tool = FunctionTool {
                kind: FunctionKind::Function,
                function,
            };
            let tools = params.tools.get_or_insert_with(Vec::new);
            tools.push(ToolParam::ResponseFormat(tool));
            if policy == ChoicePolicy::Enforce {
                params.tool_choice = Some(ToolChoice::ResponseFormat(FunctionChoice {
                    kind: FunctionKind::Function,
                    function: ChoiceFunction {
                        name: RESPONSE_FORMAT_TOOL,
                    },
                }));
            }
            params.json_mode = Some(true);
        }
        (Schema::Truthy(_) | Schema::Falsy, FormatSupport::Native) => {
            params.response_format = Some(format.value);
        }
        (Schema::Falsy, FormatSupport::ToolCall) => {}
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SystemRole {
    System,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum ProviderMessage<Field> {
    System {
        role: SystemRole,
        content: Option<Field>,
    },
    Unchanged(Record<Field>),
}

pub fn developer_to_system<Field>(
    messages: Vec<Record<Field>>,
) -> Result<Vec<ProviderMessage<Field>>, ContextError>
where
    Field: PartialEq<str>,
{
    let mut projected = Vec::with_capacity(messages.len());
    for (index, mut message) in messages.into_iter().enumerate() {
        let role = message
            .get("role")
            .ok_or(ContextError::MissingRole { index })?;
        if role.as_ref().is_some_and(|name| name == "developer") {
            tracing::debug!("Translating developer role to system role for non-OpenAI providers.");
            let content = message
                .remove("content")
                .ok_or(ContextError::MissingContent { index })?;
            projected.push(ProviderMessage::System {
                role: SystemRole::System,
                content,
            });
        } else {
            projected.push(ProviderMessage::Unchanged(message));
        }
    }
    Ok(projected)
}

pub trait ToolCallProjection<Field> {
    type Output;

    fn custom(&self, fields: Record<Field>) -> Self::Output;

    fn function(&self, fields: Record<Field>) -> Self::Output;
}

pub fn project_tool_call<Field, Projection>(
    mut record: Record<Field>,
    projection: &Projection,
) -> Projection::Output
where
    Field: PartialEq<str>,
    Projection: ToolCallProjection<Field>,
{
    let typed_custom = record
        .get("type")
        .and_then(Option::as_ref)
        .is_some_and(|kind| kind == "custom");
    let has_custom = record.get("custom").is_some_and(Option::is_some);
    if typed_custom || has_custom {
        record.retain(|key, field| field.is_some() || (key != "function" && key != "type"));
        projection.custom(record)
    } else {
        projection.function(record)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamState<Stream, Response> {
    // LiteLLM's streaming_response and response_iterator are two names for this one handle
    pub stream: Stream,
    pub json_mode: Option<bool>,
    pub http_response: Option<Response>,
}

impl<Stream, Response> From<Stream> for StreamState<Stream, Response> {
    fn from(stream: Stream) -> Self {
        Self {
            stream,
            json_mode: Some(false),
            http_response: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BatchConfigBase;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChatCompletionId(pub Uuid);

impl std::fmt::Display for ChatCompletionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "chatcmpl-{}", self.0)
    }
}
