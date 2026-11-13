use std::convert::Infallible;
use std::future::{Future, ready};
use std::sync::Arc;

use async_trait::async_trait;
use betterlite_anthropic::native_compaction::anthropic::ModelInfoLookup;
use betterlite_anthropic::native_compaction::protocol::{
    CompactionParams, NativeCompactionProvider, get_native_compaction_provider,
};
use betterlite_models::catalog::{BundledCatalog, ModelCatalog};
use betterlite_router::compaction_history::blocks::field_text;
use betterlite_router::compaction_history::compactor::{
    CompactionCounter, CompactorConfig, compactor_input, compactor_model,
};
use betterlite_router::compaction_history::contracts::{
    Candidate, Conversation, Deployment, ModelLimits, NativeSupport, ParentPayload, SummaryRequest,
    Surface,
};
use betterlite_router::compaction_history::error::{CompactionError, ErrorKind};
use betterlite_router::compaction_history::history::portable_history;
use tokio::runtime::Builder;

use super::wire::{DeploymentInput, FailureKind, Function, Input, Outcome, Output};
use crate::router_probes::error::OperationError;

struct CaseCounter {
    value: i64,
}

#[async_trait]
impl CompactionCounter for CaseCounter {
    async fn count(&self, _request: &SummaryRequest) -> Result<i64, CompactionError> {
        Ok(self.value)
    }
}

struct CatalogLookup {
    catalog: Arc<ModelCatalog>,
}

impl ModelInfoLookup for CatalogLookup {
    type Error = Infallible;

    fn supports_anthropic_compaction(
        &self,
        model: &str,
        _custom_llm_provider: &str,
    ) -> impl Future<Output = Result<Option<bool>, Infallible>> + Send {
        let flag = self
            .catalog
            .lookup
            .get(model)
            .and_then(|canonical| self.catalog.entries.get(canonical.as_ref()))
            .and_then(|entry| {
                entry
                    .fields
                    .capabilities
                    .reasoning
                    .supports_anthropic_compaction
            });
        ready(Ok(flag))
    }
}

pub(super) fn output(input: &Input) -> Result<Output, OperationError> {
    match input.function {
        Function::History => history_output(input),
        Function::Compactor => {
            let runtime = Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|source| OperationError::InvalidRequest {
                    message: format!("the probe runtime could not start: {source}"),
                })?;
            Ok(runtime.block_on(compactor_output(input)))
        }
    }
}

fn failure_kind(kind: ErrorKind) -> FailureKind {
    match kind {
        ErrorKind::Unavailable => FailureKind::ContextWindowExceeded,
        ErrorKind::Rejected => FailureKind::BadRequest,
    }
}

fn history_failure(error: &CompactionError) -> Output {
    Output::History {
        outcome: Outcome::Unavailable,
        instructions: 0,
        prefix: 0,
        tail: 0,
        calls: Vec::new(),
        results: Vec::new(),
        reason: Some(error.reason.clone()),
        status: Some(400),
        code: Some("400".to_owned()),
        kind: Some(failure_kind(error.kind)),
    }
}

fn history_output(input: &Input) -> Result<Output, OperationError> {
    let model = input.model.clone().unwrap_or_default();
    let surface = input
        .surface
        .ok_or_else(|| OperationError::InvalidRequest {
            message: "history needs a surface".to_owned(),
        })?;
    let history_input = match surface {
        Surface::Chat | Surface::Messages => Conversation::Messages(input.messages.as_slice()),
        Surface::Responses => Conversation::Responses {
            items: input.items.as_slice(),
            tools: input.tools.as_slice(),
        },
    };
    let conversation = input
        .text
        .as_deref()
        .map_or(history_input, Conversation::Text);
    match portable_history(&conversation, &model) {
        Ok(history) => Ok(Output::History {
            outcome: Outcome::Ok,
            instructions: history.instruction_count,
            prefix: history.prefix_count,
            tail: history.tail_count,
            calls: history.calls,
            results: history.results,
            reason: None,
            status: None,
            code: None,
            kind: None,
        }),
        Err(error) => Ok(history_failure(&error)),
    }
}

fn compactor_failure(error: &CompactionError, request: Option<SummaryRequest>) -> Output {
    Output::Compactor {
        outcome: Outcome::Unavailable,
        model: None,
        request,
        reason: Some(error.reason.clone()),
        status: Some(400),
        code: Some("400".to_owned()),
        kind: Some(failure_kind(error.kind)),
    }
}

async fn compactor_output(input: &Input) -> Output {
    let model = input.model.clone().unwrap_or_default();
    let config = CompactorConfig {
        model: input.config_model.as_deref(),
        max_tokens: input.max_tokens.unwrap_or_default(),
    };
    let messages = input.messages.as_slice();
    let history = match portable_history(&Conversation::Messages(messages), &model) {
        Ok(history) => history,
        Err(error) => return compactor_failure(&error, None),
    };
    let payload = ParentPayload {
        model: &model,
        messages,
        tools: None,
        system: None,
        user: None,
    };
    let request = compactor_input(&payload, &history, config.max_tokens);
    let groups = candidates(input).await;
    let counter = CaseCounter {
        value: input.count.unwrap_or_default(),
    };
    match compactor_model(&config, &groups, &request, &counter).await {
        Ok(selection) => Output::Compactor {
            outcome: Outcome::Ok,
            model: Some(selection.model),
            request: Some(request),
            reason: None,
            status: None,
            code: None,
            kind: None,
        },
        Err(error) => compactor_failure(&error, Some(request)),
    }
}

async fn candidates(input: &Input) -> Vec<Candidate> {
    let mut bundled = BundledCatalog::default();
    let catalog = bundled.get();
    let lookup = CatalogLookup { catalog };
    let mut candidates = Vec::new();
    for group in &input.groups {
        let mut deployments = Vec::new();
        for deployment in &group.deployments {
            deployments.push(deployment_candidate(&lookup, deployment).await);
        }
        candidates.push(Candidate {
            model: group.model.clone(),
            deployments,
        });
    }
    candidates
}

async fn deployment_candidate(lookup: &CatalogLookup, deployment: &DeploymentInput) -> Deployment {
    let model = field_text(&deployment.litellm_params, "model").unwrap_or_default();
    let custom_llm_provider = field_text(&deployment.litellm_params, "custom_llm_provider");
    let params = CompactionParams {
        model,
        custom_llm_provider,
    };
    let provider = get_native_compaction_provider(lookup, &params).await;
    let supports = match &provider {
        Some(provider) => provider.supports_native_compaction(&params).await,
        None => false,
    };
    let compatible_defaults = provider
        .as_ref()
        .is_some_and(|provider| provider.compatible_defaults(&deployment.litellm_params));
    Deployment {
        model: model.to_owned(),
        custom_llm_provider: custom_llm_provider.map(str::to_owned),
        limits: ModelLimits {
            max_input_tokens: deployment.max_input_tokens,
            max_output_tokens: deployment.max_output_tokens,
        },
        native: NativeSupport {
            provider: provider.is_some(),
            supports,
            compatible_defaults,
        },
    }
}
