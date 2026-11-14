use std::collections::BTreeMap;

use betterlite_models::decisions::Json as ParametersJson;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::builtin::Field;
use betterlite_prompts::factory::custom::{CustomPromptTemplate, response_schema_prompt};
use betterlite_prompts::media::inline::RemoteMediaFetcher;
use betterlite_prompts::message_content::{Content as MessageContent, Message};
use serde::Serialize;
use serde_with::skip_serializing_none;

use crate::gemini_history::content::Content;
use crate::gemini_history::convert::convert_messages_with_history;
use crate::gemini_media::equivalence::equivalent_key;
use crate::gemini_media::resolution::{convert_detail, max_media_resolution};
use crate::model_policy::capabilities::response_schema::get_supports_response_schema;
use crate::model_policy::capabilities::{
    ModelCapabilityLookup, VertexCapabilityProvider, get_supports_system_message,
};
use crate::request_labels::pop_vertex_request_labels;
use crate::studio_chat;
use crate::system_instruction::{SystemExtraction, SystemInstruction, extract_system_instruction};

use super::error::{BodyError, encode};
use super::merge::{merge_extra_body, rewrite_google_maps_response_format};

const USER: &str = "user";
const GEMINI_TWO_MARKER: &str = "gemini-2";
const LITELLM_PARAM_PREFIX: &str = "litellm_param_";
const INCLUDE_SERVER_SIDE: &str = "includeServerSideToolInvocations";

// LiteLLM's GenerationConfig TypedDict, in the order its annotations declare the fields
const GENERATION_CONFIG_FIELDS: [&str; 20] = [
    "temperature",
    "top_p",
    "top_k",
    "candidate_count",
    "max_output_tokens",
    "stop_sequences",
    "presence_penalty",
    "frequency_penalty",
    "response_mime_type",
    "response_schema",
    "response_json_schema",
    "responseFormat",
    "seed",
    "responseLogprobs",
    "logprobs",
    "responseModalities",
    "imageConfig",
    "thinkingConfig",
    "mediaResolution",
    "speechConfig",
];

pub struct BodyInput<'request> {
    pub provider: VertexCapabilityProvider,
    pub model: &'request str,
    pub messages: &'request [Message],
    pub optional_params: &'request Object,
    pub litellm_params: &'request Object,
    pub cached_content: Option<&'request str>,
    pub modify_params: bool,
    pub schema_template: Option<&'request CustomPromptTemplate>,
}

#[derive(Debug)]
pub struct Transformed {
    pub body: Object,
    // LiteLLM moves litellm_param_* entries into the caller's call metadata, which later stages read
    pub litellm_params: Object,
}

#[skip_serializing_none]
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RequestBody {
    contents: Vec<Content>,
    // LiteLLM's RequestBody keeps this one snake_case beside the camelCase keys
    #[serde(rename = "system_instruction")]
    system_instruction: Option<SystemInstruction>,
    tools: Option<Json>,
    tool_config: Option<Json>,
    safety_settings: Option<Json>,
    generation_config: Option<Object>,
    cached_content: Option<String>,
    service_tier: Option<String>,
    labels: Option<BTreeMap<String, String>>,
}

struct Pieces {
    system_instruction: Option<SystemInstruction>,
    tools: Option<Json>,
    tool_config: Option<Json>,
    safety_settings: Option<Json>,
    generation_config: Option<Object>,
    service_tier: Option<String>,
    labels: Option<BTreeMap<String, String>>,
}

pub async fn transform<Source>(
    input: &BodyInput<'_>,
    capabilities: &dyn ModelCapabilityLookup,
    fetcher: &Source,
) -> Result<Transformed, BodyError>
where
    Source: RemoteMediaFetcher + Sync,
{
    let supports_system_message =
        get_supports_system_message(input.model, input.provider, capabilities);
    let SystemExtraction {
        instruction,
        mut messages,
    } = extract_system_instruction(supports_system_message, input.messages);
    let mut optional_params = input.optional_params.clone();
    let mut litellm_params = input.litellm_params.clone();
    append_response_schema_prompt(&mut messages, &mut optional_params, input, capabilities)?;
    move_litellm_params(&mut optional_params, &mut litellm_params);
    let history = match input.provider {
        VertexCapabilityProvider::Gemini => {
            studio_chat::transform_messages(&messages, input.model, fetcher).await?
        }
        VertexCapabilityProvider::VertexAi | VertexCapabilityProvider::VertexAiBeta => {
            convert_messages_with_history(&messages, Some(input.model), fetcher).await?
        }
    };
    let config = generation_config(&optional_params, &messages, input.model)?;
    let pieces = extract_pieces(
        input,
        instruction,
        config,
        &mut optional_params,
        &litellm_params,
    )?;
    let built = RequestBody {
        contents: history.contents,
        system_instruction: pieces.system_instruction,
        tools: pieces.tools,
        tool_config: pieces.tool_config,
        safety_settings: pieces.safety_settings,
        generation_config: pieces.generation_config,
        cached_content: input.cached_content.map(str::to_owned),
        service_tier: pieces.service_tier,
        labels: pieces.labels,
    };
    let mut body: Object = encode(&built)?;
    merge_extra_body(&mut body, &mut optional_params)?;
    rewrite_google_maps_response_format(&mut body);
    Ok(Transformed {
        body,
        litellm_params,
    })
}

// LiteLLM's response_schema fallback: a model without native support gets the schema as a user prompt instead
fn append_response_schema_prompt(
    messages: &mut Vec<Message>,
    optional_params: &mut Object,
    input: &BodyInput<'_>,
    capabilities: &dyn ModelCapabilityLookup,
) -> Result<(), BodyError> {
    let Some(schema) = optional_params
        .get("response_schema")
        .filter(|value| !matches!(value, Json::Null))
        .cloned()
    else {
        return Ok(());
    };
    let supported =
        get_supports_response_schema(input.model, input.provider, capabilities).unwrap_or(false);
    if supported {
        return Ok(());
    }
    let schema: Field = encode(&schema)?;
    let prompt = response_schema_prompt(&schema, input.schema_template)?;
    messages.push(Message {
        role: USER.to_owned(),
        content: Some(MessageContent::Text(prompt)),
        name: None,
        provider_specific_fields: Object::new(),
    });
    optional_params.shift_remove("response_schema");
    Ok(())
}

// the mapped params hand the auth and project keys back to the call through this prefix
fn move_litellm_params(optional_params: &mut Object, litellm_params: &mut Object) {
    let moved: Vec<String> = optional_params
        .keys()
        .filter(|key| key.starts_with(LITELLM_PARAM_PREFIX))
        .cloned()
        .collect();
    for key in moved {
        if let Some(value) = optional_params.shift_remove(&key) {
            litellm_params.insert(key, value);
        }
    }
}

fn generation_config(
    optional_params: &Object,
    messages: &[Message],
    model: &str,
) -> Result<Object, BodyError> {
    let available: Object = GENERATION_CONFIG_FIELDS
        .iter()
        .map(|field| ((*field).to_owned(), Json::Null))
        .collect();
    let mut config: Object = optional_params
        .iter()
        .filter(|(key, _value)| equivalent_key(key, &available).is_some())
        .filter(|(_key, value)| !matches!(value, Json::Null))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    if model.contains(GEMINI_TWO_MARKER)
        && let Some(detail) = max_media_resolution(messages)
        && let Some(resolution) = convert_detail(detail)
    {
        config.insert("mediaResolution".to_owned(), encode(&resolution.level)?);
    }
    Ok(config)
}

// LiteLLM keeps the caller's spelling of a GenerationConfig key and only checks that some spelling matches
fn extract_pieces(
    input: &BodyInput<'_>,
    instruction: Option<SystemInstruction>,
    config: Object,
    optional_params: &mut Object,
    litellm_params: &Object,
) -> Result<Pieces, BodyError> {
    let tools = optional_params.shift_remove("tools");
    let tool_choice = optional_params.shift_remove("tool_choice");
    let include_server_side = include_server_side_flag(optional_params)?;
    let safety_settings = optional_params
        .shift_remove("safety_settings")
        .filter(|value| !matches!(value, Json::Null));
    optional_params.shift_remove("output_config");
    let tier = service_tier(optional_params.shift_remove("service_tier"))?;
    let labels = labels(optional_params, litellm_params, input.provider)?;
    // Vertex rejects system_instruction, tools and toolConfig beside cachedContent; dropping them is the opt-in mutation
    let can_send = input.cached_content.is_none() || !input.modify_params;
    let mut tool_config = tool_choice.filter(|_value| can_send);
    if can_send && include_server_side {
        let mut config = match tool_config.take() {
            Some(Json::Object(fields)) => fields,
            None => Object::new(),
            Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
                return Err(BodyError::Field {
                    field: "tool_choice",
                    expected: "an object",
                });
            }
        };
        config.insert(INCLUDE_SERVER_SIDE.to_owned(), Json::Bool(true));
        tool_config = Some(Json::Object(config));
    }
    Ok(Pieces {
        system_instruction: instruction.filter(|_instruction| can_send),
        tools: tools.filter(|_tools| can_send),
        tool_config,
        safety_settings,
        generation_config: (!config.is_empty()).then_some(config),
        service_tier: tier,
        labels,
    })
}

fn include_server_side_flag(optional_params: &mut Object) -> Result<bool, BodyError> {
    match optional_params.shift_remove("include_server_side_tool_invocations") {
        None | Some(Json::Null | Json::Bool(false)) => Ok(false),
        Some(Json::Bool(true)) => Ok(true),
        Some(Json::Number(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            Err(BodyError::Field {
                field: "include_server_side_tool_invocations",
                expected: "a boolean",
            })
        }
    }
}

fn service_tier(tier: Option<Json>) -> Result<Option<String>, BodyError> {
    match tier {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => Ok(Some(if text.eq_ignore_ascii_case("default") {
            "standard".to_owned()
        } else {
            text.to_lowercase()
        })),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(BodyError::Field {
                field: "service_tier",
                expected: "a string",
            })
        }
    }
}

fn labels(
    optional_params: &Object,
    litellm_params: &Object,
    provider: VertexCapabilityProvider,
) -> Result<Option<BTreeMap<String, String>>, BodyError> {
    let mut params: ParametersJson = encode(optional_params)?;
    let metadata: ParametersJson = encode(litellm_params)?;
    let labels = pop_vertex_request_labels(&mut Some(params), Some(&metadata));
    Ok(labels.filter(|_labels| provider != VertexCapabilityProvider::Gemini))
}
