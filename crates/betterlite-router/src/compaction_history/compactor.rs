use async_trait::async_trait;
use betterlite_models::decisions::Json;
use betterlite_openai::chat::{ChatMessage, Tool};

use super::budget::{COMPACTOR_BUDGET_RATIO, budget};
use super::contracts::{
    Candidate, History, NativeCapability, NativePrefix, OutputAllowance, ParentPayload,
    ParentTools, ResponsesPayload, Selection, SummaryRequest, SummaryTools,
};
use super::error::{CompactionError, ErrorKind};

pub struct CompactorConfig<'config> {
    pub model: Option<&'config str>,
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

pub struct ResponsesInput<'params> {
    pub payload: &'params ResponsesPayload<'params>,
    pub converter: &'params dyn ResponsesPrefix,
}

pub enum ParentInput<'params> {
    Chat(&'params ParentPayload<'params>),
    Responses(&'params ResponsesInput<'params>),
}

fn parent_tools(tools: ParentTools<'_>) -> SummaryTools {
    match tools {
        ParentTools::Chat(chat) => SummaryTools::Chat(chat.to_vec()),
        ParentTools::Anthropic(anthropic) => SummaryTools::Anthropic(anthropic.to_vec()),
    }
}

fn older_items<'items, Entry>(conversation: &'items [Entry], history: &History) -> &'items [Entry] {
    conversation
        .get(
            ..history
                .instruction_count
                .saturating_add(history.prefix_count),
        )
        .unwrap_or(conversation)
}

pub fn compactor_input(
    parent: &ParentInput<'_>,
    history: &History,
    output: i64,
) -> Result<SummaryRequest, CompactionError> {
    match parent {
        ParentInput::Chat(payload) => Ok(SummaryRequest {
            model: payload.model.to_owned(),
            messages: older_items(payload.messages, history).to_vec(),
            max_tokens: output,
            tools: payload.tools.map(parent_tools),
            system: payload.system.cloned(),
            user: payload.user.cloned(),
        }),
        ParentInput::Responses(input) => {
            let payload = input.payload;
            let older = older_items(payload.items, history);
            let prefix = native_prefix(older, payload.tools, payload.request, input.converter)?;
            Ok(SummaryRequest {
                model: payload.model.to_owned(),
                messages: prefix.messages,
                max_tokens: output,
                tools: Some(SummaryTools::Chat(prefix.tools)),
                system: payload.system.cloned(),
                user: payload.user.cloned(),
            })
        }
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

pub async fn compactor_model(
    config: &CompactorConfig<'_>,
    candidates: &[Candidate],
    request: &SummaryRequest,
    counter: &dyn CompactionCounter,
) -> Result<Selection, CompactionError> {
    let needed = counter.count(request).await?;
    let found = config.model.map_or_else(
        || first_eligible(candidates, request, needed),
        |model| {
            candidates
                .iter()
                .find(|candidate| candidate.model == model)
                .map(|candidate| eligible(candidate, request, needed))
                .transpose()
                .map(Option::flatten)
        },
    );
    found?.ok_or_else(|| CompactionError {
        kind: ErrorKind::Unavailable,
        model: request.model.clone(),
        reason:
            "No configured compactor supports native compaction with enough context and compatible defaults"
                .to_owned(),
        source: None,
    })
}

fn first_eligible(
    candidates: &[Candidate],
    request: &SummaryRequest,
    needed: i64,
) -> Result<Option<Selection>, CompactionError> {
    for candidate in candidates {
        if let Some(selection) = eligible(candidate, request, needed)? {
            return Ok(Some(selection));
        }
    }
    Ok(None)
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
        if deployment.native.capability != NativeCapability::Supported {
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
