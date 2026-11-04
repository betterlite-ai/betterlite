use serde_json::value::to_raw_value;

use super::wire::{
    Citation, Content, Delta, Function, ProviderFields, Record, ToolCall, UsageData,
};
use super::{CohereChunk, CohereStreamError, CohereUsage};

pub(super) fn parse_v2(record: Record, chunk: &mut CohereChunk) -> Result<(), CohereStreamError> {
    match record.kind.as_deref() {
        Some("content-delta" | "tool-call-delta") => {
            let delta: Option<Delta> = record
                .delta
                .as_ref()
                .map(|raw| serde_json::from_str(raw.get()))
                .transpose()?;
            if record.kind.as_deref() == Some("content-delta") {
                chunk.text = match delta
                    .and_then(|value| value.message)
                    .and_then(|message| message.content)
                {
                    Some(Content::Text(text) | Content::Object { text: Some(text) }) => text,
                    Some(Content::Object { text: None }) | None => String::new(),
                };
            } else {
                chunk.tool_use = delta.and_then(tool_delta);
            }
        }
        _ => parse_event(&record, chunk)?,
    }
    if let Some(citations) = record.citations {
        chunk
            .provider_specific_fields
            .get_or_insert(ProviderFields {
                tool_plan: None,
                citations: None,
            })
            .citations = Some(citations);
    }
    Ok(())
}

fn tool_delta(delta: Delta) -> Option<ToolCall> {
    delta
        .tool_calls
        .and_then(|tools| tools.into_iter().next())
        .map(|tool| ToolCall {
            id: tool.id.unwrap_or_default(),
            kind: "function",
            function: Function {
                name: tool.name.unwrap_or_default(),
                arguments: tool.arguments.unwrap_or_default(),
            },
        })
}

fn parse_event(record: &Record, chunk: &mut CohereChunk) -> Result<(), CohereStreamError> {
    let delta = record.data.as_ref().and_then(|data| data.delta.as_ref());
    let message = delta.and_then(|value| value.message.as_ref());
    if record.event.as_deref() == Some("message-end") {
        chunk.is_finished = true;
        chunk.finish_reason = delta
            .and_then(|value| value.finish_reason.clone())
            .unwrap_or_else(|| String::from("stop"));
        chunk.usage = delta
            .and_then(|value| value.usage.as_ref())
            .map(usage)
            .transpose()?
            .flatten();
        return Ok(());
    }
    chunk.provider_specific_fields = match record.event.as_deref() {
        Some("tool-plan-delta") => message
            .and_then(|value| value.tool_plan.as_ref())
            .filter(|plan| !plan.is_empty())
            .map(|plan| ProviderFields {
                tool_plan: Some(plan.clone()),
                citations: None,
            }),
        Some("citation-start") => message
            .and_then(|value| value.citations.as_ref())
            .map(citation_fields)
            .transpose()?
            .flatten(),
        _ => None,
    };
    Ok(())
}

fn citation_fields(citation: &Citation) -> Result<Option<ProviderFields>, CohereStreamError> {
    if citation.start.is_none()
        && citation.end.is_none()
        && citation.text.is_none()
        && citation.sources.is_none()
        && citation.kind.is_none()
    {
        return Ok(None);
    }
    let normalized = Citation {
        start: Some(citation.start.unwrap_or(0)),
        end: Some(citation.end.unwrap_or(0)),
        text: Some(citation.text.clone().unwrap_or_default()),
        sources: Some(citation.sources.clone().unwrap_or_default()),
        kind: Some(
            citation
                .kind
                .clone()
                .unwrap_or_else(|| String::from("TEXT_CONTENT")),
        ),
    };
    Ok(Some(ProviderFields {
        tool_plan: None,
        citations: Some(to_raw_value(&[normalized])?),
    }))
}

fn usage(data: &UsageData) -> Result<Option<CohereUsage>, CohereStreamError> {
    if data.tokens.is_none() && data.billed_units.is_none() {
        return Ok(None);
    }
    let prompt = data
        .tokens
        .as_ref()
        .and_then(|tokens| tokens.input_tokens)
        .unwrap_or(0);
    let completion = data
        .tokens
        .as_ref()
        .and_then(|tokens| tokens.output_tokens)
        .unwrap_or(0);
    Ok(Some(CohereUsage {
        prompt_tokens: prompt,
        completion_tokens: completion,
        total_tokens: prompt
            .checked_add(completion)
            .ok_or(CohereStreamError::UsageOverflow)?,
    }))
}
