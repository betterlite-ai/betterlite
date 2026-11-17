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
use crate::gemini_media::gcs_metadata::GcsDeps;
use crate::gemini_media::resolution::{convert_detail, max_media_resolution};
use crate::model_policy::capabilities::response_schema::get_supports_response_schema;
use crate::model_policy::capabilities::{
    ModelCapabilityLookup, VertexCapabilityProvider, get_supports_system_message,
};
use crate::request_labels::pop_vertex_request_labels;
use crate::studio_chat;
use crate::system_instruction::{SystemExtraction, SystemInstruction, extract_system_instruction};

use super::error::{BodyError, encode};
use super::media_prepare::resolve_gcs_content_types;
use super::merge::{merge_extra_body, rewrite_maps_response_format};

const INCLUDE_SERVER_SIDE: &str = "includeServerSideToolInvocations";
const GENERATION_CONFIG_FIELDS: &str = "temperature top_p top_k candidate_count max_output_tokens stop_sequences presence_penalty frequency_penalty response_mime_type response_schema response_json_schema responseFormat seed responseLogprobs logprobs responseModalities imageConfig thinkingConfig mediaResolution speechConfig";

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
    pub litellm_params: Object,
}

#[skip_serializing_none]
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RequestBody {
    contents: Vec<Content>,
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

struct Prepared {
    instruction: Option<SystemInstruction>,
    messages: Vec<Message>,
    params: Object,
    litellm_params: Object,
}

// LiteLLM's _transform_request_body order: system extraction, the schema prompt, then the litellm_param_ keys
fn prepare_body(
    input: &BodyInput<'_>,
    capabilities: &(dyn ModelCapabilityLookup + Sync),
) -> Result<Prepared, BodyError> {
    let SystemExtraction {
        instruction,
        mut messages,
    } = extract_system_instruction(
        get_supports_system_message(input.model, input.provider, capabilities),
        input.messages,
    );
    let mut params = input.optional_params.clone();
    let mut litellm_params = input.litellm_params.clone();
    append_response_schema_prompt(&mut messages, &mut params, input, capabilities)?;
    litellm_params.extend(params.extract_if(.., |key, _value| key.starts_with("litellm_param_")));
    Ok(Prepared {
        instruction,
        messages,
        params,
        litellm_params,
    })
}

pub async fn transform<Source>(
    input: &BodyInput<'_>,
    capabilities: &(dyn ModelCapabilityLookup + Sync),
    fetcher: &Source,
) -> Result<Transformed, BodyError>
where
    Source: RemoteMediaFetcher + Sync,
{
    // the oracle adapters and any caller without a GCS client convert without the metadata pass
    convert_body(input, prepare_body(input, capabilities)?, fetcher).await
}

impl BodyInput<'_> {
    pub async fn transform_with_gcs<Source>(
        &self,
        gcs: &GcsDeps<'_>,
        capabilities: &(dyn ModelCapabilityLookup + Sync),
        fetcher: &Source,
    ) -> Result<Transformed, BodyError>
    where
        Source: RemoteMediaFetcher + Sync,
    {
        let mut prepared = prepare_body(self, capabilities)?;
        prepared.messages =
            resolve_gcs_content_types(prepared.messages, &prepared.litellm_params, gcs).await?;
        convert_body(self, prepared, fetcher).await
    }
}

async fn convert_body<Source>(
    input: &BodyInput<'_>,
    prepared: Prepared,
    fetcher: &Source,
) -> Result<Transformed, BodyError>
where
    Source: RemoteMediaFetcher + Sync,
{
    let Prepared {
        instruction,
        messages,
        mut params,
        litellm_params,
    } = prepared;
    let history = match input.provider {
        VertexCapabilityProvider::Gemini => {
            studio_chat::transform_messages(&messages, input.model, fetcher).await?
        }
        VertexCapabilityProvider::VertexAi | VertexCapabilityProvider::VertexAiBeta => {
            convert_messages_with_history(&messages, Some(input.model), fetcher).await?
        }
    };
    if input.model.contains("gemini-2")
        && let Some(detail) = max_media_resolution(&messages)
        && let Some(resolution) = convert_detail(detail)
    {
        params.insert("mediaResolution".to_owned(), encode(&resolution.level)?);
    }
    assemble(
        input,
        instruction,
        history.contents,
        &mut params,
        litellm_params,
    )
}

fn assemble(
    input: &BodyInput<'_>,
    instruction: Option<SystemInstruction>,
    contents: Vec<Content>,
    params: &mut Object,
    litellm_params: Object,
) -> Result<Transformed, BodyError> {
    let can_send = input.cached_content.is_none() || !input.modify_params;
    let config = generation_config(params);
    let tools = params
        .shift_remove("tools")
        .filter(|value| !matches!(value, Json::Null));
    let tool_choice = params.shift_remove("tool_choice");
    let tool_config = tool_config(params, tool_choice, can_send)?;
    let safety_settings = params
        .shift_remove("safety_settings")
        .filter(|value| !matches!(value, Json::Null));
    params.shift_remove("output_config");
    let typed = encode::<Object, ParametersJson>(params)?;
    let metadata: ParametersJson = encode(&litellm_params)?;
    let labels = pop_vertex_request_labels(&mut Some(typed), Some(&metadata))
        .filter(|_labels| input.provider != VertexCapabilityProvider::Gemini);
    let built = RequestBody {
        contents,
        system_instruction: instruction.filter(|_instruction| can_send),
        tools: tools.filter(|_tools| can_send),
        tool_config,
        safety_settings,
        generation_config: (!config.is_empty()).then_some(config),
        cached_content: input.cached_content.map(str::to_owned),
        service_tier: service_tier(params.shift_remove("service_tier"))?,
        labels,
    };
    let mut body: Object = encode(&built)?;
    merge_extra_body(&mut body, params)?;
    rewrite_maps_response_format(&mut body);
    Ok(Transformed {
        body,
        litellm_params,
    })
}

fn tool_config(
    params: &mut Object,
    tool_choice: Option<Json>,
    can_send: bool,
) -> Result<Option<Json>, BodyError> {
    let include: bool = match params.shift_remove("include_server_side_tool_invocations") {
        None | Some(Json::Null) => false,
        Some(value) => encode(&value).map_err(|_error| {
            BodyError::Field("include_server_side_tool_invocations", "a boolean")
        })?,
    };
    let mut config = match params.shift_remove("toolConfig") {
        None | Some(Json::Null) => Object::new(),
        Some(Json::Object(fields)) => fields,
        Some(_) => return Err(BodyError::Field("toolConfig", "an object")),
    };
    if !can_send {
        return Ok(None);
    }
    let choice = tool_choice.filter(|value| !matches!(value, Json::Null));
    let supplied = matches!(choice, Some(Json::Object(_)));
    match choice {
        Some(Json::Object(fields)) => config.extend(fields),
        Some(value) if !include => return Ok(Some(value)),
        Some(_) => return Err(BodyError::Field("tool_choice", "an object")),
        None => {}
    }
    if include {
        config.insert(INCLUDE_SERVER_SIDE.to_owned(), Json::Bool(true));
    }
    Ok((supplied || include || !config.is_empty()).then_some(Json::Object(config)))
}

// LiteLLM's fallback for a model without native schema support: the schema becomes a user prompt
fn append_response_schema_prompt(
    messages: &mut Vec<Message>,
    params: &mut Object,
    input: &BodyInput<'_>,
    capabilities: &dyn ModelCapabilityLookup,
) -> Result<(), BodyError> {
    let candidate = params
        .get("response_schema")
        .filter(|value| !matches!(value, Json::Null))
        .cloned()
        .filter(|_schema| {
            !get_supports_response_schema(input.model, input.provider, capabilities)
                .unwrap_or(false)
        });
    let Some(schema) = candidate else {
        return Ok(());
    };
    let field: Field = encode(&schema)?;
    let prompt = response_schema_prompt(&field, input.schema_template)?;
    messages.push(Message {
        role: "user".to_owned(),
        content: Some(MessageContent::Text(prompt)),
        name: None,
        provider_specific_fields: Object::new(),
    });
    params.shift_remove("response_schema");
    Ok(())
}

fn generation_config(params: &Object) -> Object {
    let available: Object = GENERATION_CONFIG_FIELDS
        .split_whitespace()
        .map(|field| (field.to_owned(), Json::Null))
        .collect();
    params
        .iter()
        .filter(|(key, _value)| equivalent_key(key.as_str(), &available).is_some())
        .filter(|(_key, value)| !matches!(value, Json::Null))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

fn service_tier(tier: Option<Json>) -> Result<Option<String>, BodyError> {
    match tier {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) if text.eq_ignore_ascii_case("default") => {
            Ok(Some("standard".to_owned()))
        }
        Some(Json::Text(text)) => Ok(Some(text.to_lowercase())),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(BodyError::Field("service_tier", "a string"))
        }
    }
}
