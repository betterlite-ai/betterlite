use async_trait::async_trait;
use betterlite_models::decisions::Json;
use betterlite_openai::chat::{ChatMessage, Tool};

use super::budget::{COMPACTOR_BUDGET_RATIO, budget};
use super::contracts::{
    Candidate, History, NativePrefix, OutputAllowance, ParentPayload, ResponsesPayload, Selection,
    SummaryRequest,
};
use super::error::{CompactionError, ErrorKind};

pub struct CompactorConfig<'a> {
    pub model: Option<&'a str>,
    pub max_tokens: i64,
}

#[async_trait]
pub trait CompactionCounter: Send + Sync {
    async fn count(&self, request: &SummaryRequest) -> Result<i64, CompactionError>;
}

pub trait ResponsesPrefix {
    fn messages(&self, items: &[Json], request: &Json)
    -> Result<Vec<ChatMessage>, CompactionError>;

    fn tools(&self, tools: &[Json]) -> Result<Vec<Tool>, CompactionError>;
}

#[must_use]
pub fn compactor_input(
    payload: &ParentPayload<'_>,
    history: &History,
    output: i64,
) -> SummaryRequest {
    let end = history
        .instruction_count
        .saturating_add(history.prefix_count);
    let older = payload.messages.get(..end).unwrap_or(payload.messages);
    SummaryRequest {
        model: payload.model.to_owned(),
        messages: older.to_vec(),
        max_tokens: output,
        tools: payload.tools.map(<[Tool]>::to_vec),
        system: payload.system.cloned(),
        user: payload.user.cloned(),
    }
}

pub fn native_prefix(
    items: &[Json],
    tools: &[Json],
    request: &Json,
    converter: &dyn ResponsesPrefix,
) -> Result<NativePrefix, CompactionError> {
    Ok(NativePrefix {
        messages: converter.messages(items, request)?,
        tools: converter.tools(tools)?,
    })
}

pub fn responses_compactor_input(
    payload: &ResponsesPayload<'_>,
    history: &History,
    output: i64,
    converter: &dyn ResponsesPrefix,
) -> Result<SummaryRequest, CompactionError> {
    let end = history
        .instruction_count
        .saturating_add(history.prefix_count);
    let older = payload.items.get(..end).unwrap_or(payload.items);
    let prefix = native_prefix(older, payload.tools, payload.request, converter)?;
    Ok(SummaryRequest {
        model: payload.model.to_owned(),
        messages: prefix.messages,
        max_tokens: output,
        tools: Some(prefix.tools),
        system: payload.system.cloned(),
        user: payload.user.cloned(),
    })
}

pub async fn compactor_model(
    config: &CompactorConfig<'_>,
    candidates: &[Candidate],
    request: &SummaryRequest,
    counter: &dyn CompactionCounter,
) -> Result<Selection, CompactionError> {
    let needed = counter.count(request).await?;
    let found = match config.model {
        Some(model) => candidates
            .iter()
            .find(|candidate| candidate.model == model)
            .map(|candidate| eligible(candidate, request, needed))
            .transpose()?
            .flatten(),
        None => {
            let mut chosen = None;
            for candidate in candidates {
                chosen = eligible(candidate, request, needed)?;
                if chosen.is_some() {
                    break;
                }
            }
            chosen
        }
    };
    match found {
        Some(selection) => Ok(selection),
        None => Err(CompactionError {
            kind: ErrorKind::Unavailable,
            model: request.model.clone(),
            reason:
                "No configured compactor supports native compaction with enough context and compatible defaults"
                    .to_owned(),
        }),
    }
}

fn eligible(
    candidate: &Candidate,
    request: &SummaryRequest,
    needed: i64,
) -> Result<Option<Selection>, CompactionError> {
    let Some(first) = candidate.deployments.first() else {
        return Ok(None);
    };
    if !first.native.provider {
        return Ok(None);
    }
    for deployment in &candidate.deployments {
        if !deployment.native.supports || !deployment.native.compatible_defaults {
            return Ok(None);
        }
        let allowance = OutputAllowance {
            max_completion_tokens: None,
            max_output_tokens: None,
            max_tokens: Some(request.max_tokens),
        };
        let limits = budget(
            &request.model,
            deployment.limits,
            allowance,
            COMPACTOR_BUDGET_RATIO,
        )?;
        let Some(available) = limits.available else {
            return Ok(None);
        };
        if needed > available {
            return Ok(None);
        }
    }
    Ok(Some(Selection {
        model: candidate.model.clone(),
        deployment: first.clone(),
    }))
}
