use betterlite_logging::verbose_hooks::{ConsoleSink, LoggerSink, VerboseHooks};

use crate::stream_base_assembly::StreamedChunks;
use crate::stream_base_assembly::provider_metadata::{
    ProviderMetadataContext, apply_provider_metadata,
};
use crate::stream_base_assembly::response::BaseResponse;
use crate::stream_chunk_shaping::records::{JsonObject, JsonValue};
use crate::stream_usage_assembly::wire::{ReasoningChoice, ReasoningMessage, Usage};
use crate::stream_usage_assembly::{UsageRequest, calculate_usage, count_reasoning_tokens};

use super::cost::{self, CostInput, RESPONSE_COST_KEY};
use super::records::Chunk;
use super::response::{ChatChoice, ChatMessage, ChatResponse, Draft};
use super::{AssemblyError, Request, content, project};

#[derive(Clone, Copy)]
pub(crate) enum ReasoningMode {
    Zero,
    Counted,
}

pub(crate) async fn finish_chat<L, C>(
    request: &Request<'_>,
    streamed: &StreamedChunks,
    mut draft: Draft,
    mode: ReasoningMode,
    hooks: &VerboseHooks<L, C>,
) -> Result<ChatResponse, AssemblyError>
where
    L: LoggerSink + Sync,
    C: ConsoleSink + Sync,
{
    let mut usage = estimate_usage(request, streamed, &draft, mode, hooks).await?;
    propagate_hidden_fields(&mut draft.base.hidden_params, &streamed.chunks)?;
    apply_costs(request, &mut draft.base, &mut usage)?;
    apply_metadata(request, streamed, &mut draft.base)?;
    build_response(draft, usage)
}

// LiteLLM hands the first sorted chunk's model to the counters, not the base model a later chunk may replace.
async fn estimate_usage<L, C>(
    request: &Request<'_>,
    streamed: &StreamedChunks,
    draft: &Draft,
    mode: ReasoningMode,
    hooks: &VerboseHooks<L, C>,
) -> Result<Usage, AssemblyError>
where
    L: LoggerSink + Sync,
    C: ConsoleSink + Sync,
{
    let output = content::completion_output(
        draft
            .base
            .choices
            .first()
            .and_then(|choice| choice.message.content.as_deref()),
        draft.extras.function_call.as_ref(),
        draft.extras.tool_calls.as_deref(),
    )?;
    let reasoning_tokens = match mode {
        ReasoningMode::Zero => Some(0),
        ReasoningMode::Counted => {
            reasoning_tokens(draft.extras.reasoning_content.as_deref(), request).await?
        }
    };
    let projected = project::usage_chunks(&streamed.chunks)?;
    let first_model = streamed
        .chunks
        .first()
        .and_then(|chunk| chunk.model.as_deref())
        .unwrap_or_default();
    let usage_request = UsageRequest {
        chunks: &projected,
        model: first_model,
        completion_output: &output,
        messages: request.messages,
        reasoning_tokens,
        count_prompt_tokens: request.count_prompt_tokens,
    };
    calculate_usage(request.counter, hooks, &usage_request)
        .await
        .map_err(AssemblyError::from)
}

fn apply_costs(
    request: &Request<'_>,
    base: &mut BaseResponse,
    usage: &mut Usage,
) -> Result<(), AssemblyError> {
    let stamp_input = CostInput {
        model: &base.model,
        usage,
        hidden_params: &base.hidden_params,
    };
    if let Some(stamped) = cost::stamp_usage_cost(&stamp_input, request.logging)? {
        usage.cost = Some(stamped);
    }
    let input = CostInput {
        model: &base.model,
        usage,
        hidden_params: &base.hidden_params,
    };
    let reported = cost::stream_builder_response_cost(&input, request.logging, request.costs)?;
    if let Some(value) = reported.and_then(cost::response_cost_value) {
        base.hidden_params
            .insert(RESPONSE_COST_KEY.to_owned(), value);
    }
    Ok(())
}

fn apply_metadata(
    request: &Request<'_>,
    streamed: &StreamedChunks,
    base: &mut BaseResponse,
) -> Result<(), AssemblyError> {
    let Some(services) = request.metadata else {
        return Ok(());
    };
    let context = ProviderMetadataContext {
        custom_llm_provider: request.logging.and_then(|logging| logging.provider()),
        redactor: services.redactor,
        resolver: services.resolver,
        selector: services.selector,
        projection: services.projection,
    };
    let chunks = project::base_chunks(&streamed.chunks)?;
    apply_provider_metadata(base, &chunks, &context);
    Ok(())
}

async fn reasoning_tokens(
    reasoning_content: Option<&str>,
    request: &Request<'_>,
) -> Result<Option<i64>, AssemblyError> {
    let choices: Vec<ReasoningChoice> = reasoning_content.map_or_else(Vec::new, |text| {
        vec![ReasoningChoice {
            message: ReasoningMessage {
                reasoning_content: Some(text.to_owned()),
            },
        }]
    });
    Ok(count_reasoning_tokens(request.counter, &choices).await?)
}

// The nearest chunk from the end whose hidden params carry provider_specific_fields merges into the response.
fn propagate_hidden_fields(
    hidden_params: &mut JsonObject,
    chunks: &[Chunk],
) -> Result<(), AssemblyError> {
    for chunk in chunks.iter().rev() {
        let Some(hidden) = chunk.hidden_object() else {
            continue;
        };
        let Some(value) = hidden.get("provider_specific_fields") else {
            continue;
        };
        let JsonValue::Object(fields) = value else {
            return Err(AssemblyError::InvalidProviderFields);
        };
        match hidden_params.get_mut("provider_specific_fields") {
            Some(JsonValue::Object(existing)) => existing.extend(fields.clone()),
            Some(_) => return Err(AssemblyError::InvalidProviderFields),
            None => {
                hidden_params.insert(
                    "provider_specific_fields".to_owned(),
                    JsonValue::Object(fields.clone()),
                );
            }
        }
        return Ok(());
    }
    Ok(())
}

fn build_response(draft: Draft, usage: Usage) -> Result<ChatResponse, AssemblyError> {
    let Draft { base, extras } = draft;
    let Some(choice) = base.choices.into_iter().next() else {
        return Err(AssemblyError::MissingChoice);
    };
    Ok(ChatResponse {
        id: base.id,
        object: base.object,
        created: base.created,
        model: base.model,
        system_fingerprint: base.system_fingerprint,
        service_tier: base.service_tier,
        choices: vec![ChatChoice {
            index: choice.index,
            message: ChatMessage {
                content: choice.message.content,
                role: choice.message.role,
                tool_calls: extras.tool_calls,
                function_call: extras.function_call,
                provider_specific_fields: extras.provider_specific_fields,
                reasoning_content: extras.reasoning_content,
                thinking_blocks: extras.thinking_blocks,
                annotations: extras.annotations,
                audio: extras.audio,
                images: extras.images,
            },
            finish_reason: choice.finish_reason,
            provider_specific_fields: choice.provider_specific_fields,
        }],
        usage,
        hidden_params: base.hidden_params,
    })
}
