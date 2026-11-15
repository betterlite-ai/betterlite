use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use indexmap::IndexMap;

use crate::generic_guardrail_contracts::{GuardrailInputType, UnreachableFallback};
use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_base::inputs::{GuardrailApiInputs, GuardrailLogTarget};
use crate::vigil_guard::analysis::{Decision, Source, analyze};
use crate::vigil_guard::block_reason::build_block_reason;
use crate::vigil_guard::error::VigilGuardError;
use crate::vigil_guard::failure::{FailureContext, handle_backend_failure};
use crate::vigil_guard::metadata::{MetadataValue, collect_metadata};
use crate::vigil_guard::output::{Inputs, ToolCall, build_output};
use crate::vigil_guard::sanitize::resolve_sanitized_text;
use crate::vigil_guard::tool_calls::{set_tool_call_arguments, tool_call_arguments};

use super::VigilGuard;
use super::bridge::{self, CallIds};

const UNRECOGNIZED_DECISION_MESSAGE: &str = "Vigil Guard returned an unrecognized decision.";
const RAISED_STATUS_CODE: u16 = 400;

pub(super) async fn apply(
    guardrail: &VigilGuard,
    inputs: GuardrailApiInputs,
    request_data: &BTreeMap<String, GuardrailValue>,
    input_type: GuardrailInputType,
    logging_obj: Option<&GuardrailLogTarget>,
) -> Result<GuardrailApiInputs, GuardrailFailure> {
    let bridged = bridge::from_inputs(&inputs, input_type)?;
    let texts = inputs.texts.clone().unwrap_or_default();
    let scanned = match input_type {
        GuardrailInputType::Response => tool_call_arguments(bridged.tool_calls.as_deref()),
        GuardrailInputType::Request => Vec::new(),
    };
    if texts.iter().all(|text| text.trim().is_empty()) && scanned.is_empty() {
        return Ok(inputs);
    }
    let source = match input_type {
        GuardrailInputType::Request => Source::UserInput,
        GuardrailInputType::Response => Source::ModelOutput,
    };
    let metadata = match logging_obj {
        Some(target) => collect_metadata(request_data, Some(&CallIds { target })),
        None => collect_metadata(request_data, None),
    };
    let run = Run {
        guardrail,
        original: &inputs,
        bridged: &bridged,
        source,
        metadata,
    };
    let rewritten = run.texts(&texts).await?;
    run.tool_calls(&rewritten, &scanned).await
}

struct Run<'run> {
    guardrail: &'run VigilGuard,
    original: &'run GuardrailApiInputs,
    bridged: &'run Inputs,
    source: Source,
    metadata: IndexMap<String, MetadataValue>,
}

enum ScanStop {
    Failed(VigilGuardError),
    Blocked(GuardrailFailure),
    Unrecognized,
}

impl Run<'_> {
    async fn scan(&self, text: &str) -> Result<String, ScanStop> {
        let analysis = match analyze(
            self.guardrail.handler.as_ref(),
            &self.guardrail.settings,
            text,
            self.source,
            &self.metadata,
        )
        .await
        {
            Ok(analysis) => analysis,
            Err(error) => return Err(ScanStop::Failed(error)),
        };
        let decision = analysis.decision.as_ref().and_then(Decision::from_value);
        match decision {
            Some(Decision::Allowed) => Ok(text.to_owned()),
            Some(Decision::Sanitized) => Ok(resolve_sanitized_text(text, &analysis)),
            Some(Decision::Blocked) => Err(ScanStop::Blocked(GuardrailFailure::GuardrailRaised {
                guardrail_name: self.guardrail.base.settings.guardrail_name.clone(),
                message: build_block_reason(&analysis),
                wrap_default_message: false,
                status_code: RAISED_STATUS_CODE,
                blocked_content: true,
            })),
            None => Err(ScanStop::Unrecognized),
        }
    }

    async fn texts(&self, texts: &[String]) -> Result<Vec<String>, GuardrailFailure> {
        let mut rewritten = Vec::with_capacity(texts.len());
        for (index, text) in texts.iter().enumerate() {
            if text.trim().is_empty() {
                rewritten.push(text.clone());
                continue;
            }
            match self.scan(text).await {
                Ok(scanned) => rewritten.push(scanned),
                Err(ScanStop::Blocked(failure)) => return Err(failure),
                Err(ScanStop::Failed(error)) => {
                    let partial = partial_texts(&rewritten, texts, index);
                    return self.interrupted(error, &partial, self.bridged.tool_calls.as_deref());
                }
                Err(ScanStop::Unrecognized) => {
                    let partial = partial_texts(&rewritten, texts, index);
                    return self.unrecognized(&partial, self.bridged.tool_calls.as_deref());
                }
            }
        }
        Ok(rewritten)
    }

    async fn tool_calls(
        &self,
        texts: &[String],
        scanned: &[(usize, String)],
    ) -> Result<GuardrailApiInputs, GuardrailFailure> {
        let mut rewritten = self.bridged.tool_calls.clone();
        for (index, arguments) in scanned {
            match self.scan(arguments).await {
                Ok(rewritten_arguments) => {
                    let current = rewritten.as_deref().unwrap_or_default();
                    let updated = set_tool_call_arguments(current, *index, &rewritten_arguments)?;
                    rewritten = Some(updated);
                }
                Err(ScanStop::Blocked(failure)) => return Err(failure),
                Err(ScanStop::Failed(error)) => {
                    return self.interrupted(error, texts, rewritten.as_deref());
                }
                Err(ScanStop::Unrecognized) => {
                    return self.unrecognized(texts, rewritten.as_deref());
                }
            }
        }
        let built = build_output(self.bridged, texts, rewritten.as_deref());
        Ok(bridge::to_output(self.original, self.bridged, built))
    }

    fn interrupted(
        &self,
        error: VigilGuardError,
        texts: &[String],
        tool_calls: Option<&[ToolCall]>,
    ) -> Result<GuardrailApiInputs, GuardrailFailure> {
        // httpx raises a LocalProtocolError for an unencodable header, so Python treats it as a backend failure
        if !matches!(
            error,
            VigilGuardError::InvalidHeader { .. }
                | VigilGuardError::Transport { .. }
                | VigilGuardError::ResponseJson { .. }
        ) {
            return Err(GuardrailFailure::from(error));
        }
        let context = FailureContext {
            fallback: self.guardrail.unreachable_fallback,
            guardrail_name: self
                .guardrail
                .base
                .settings
                .guardrail_name
                .as_deref()
                .unwrap_or("None"),
            source: self.source,
        };
        match handle_backend_failure(&context, &error, self.bridged, texts, tool_calls) {
            Ok(built) => Ok(bridge::to_output(self.original, self.bridged, built)),
            Err(backend) => Err(GuardrailFailure::GuardrailRaised {
                guardrail_name: self.guardrail.base.settings.guardrail_name.clone(),
                message: backend.to_string(),
                wrap_default_message: false,
                status_code: backend.status_code().as_u16(),
                blocked_content: false,
            }),
        }
    }

    fn unrecognized(
        &self,
        texts: &[String],
        tool_calls: Option<&[ToolCall]>,
    ) -> Result<GuardrailApiInputs, GuardrailFailure> {
        tracing::error!(
            guardrail_name = self
                .guardrail
                .base
                .settings
                .guardrail_name
                .as_deref()
                .unwrap_or("None"),
            source = ?self.source,
            "Vigil Guard returned an unrecognized decision"
        );
        match self.guardrail.unreachable_fallback {
            UnreachableFallback::FailOpen => {
                let built = build_output(self.bridged, texts, tool_calls);
                Ok(bridge::to_output(self.original, self.bridged, built))
            }
            UnreachableFallback::FailClosed => Err(GuardrailFailure::GuardrailRaised {
                guardrail_name: self.guardrail.base.settings.guardrail_name.clone(),
                message: UNRECOGNIZED_DECISION_MESSAGE.to_owned(),
                wrap_default_message: false,
                status_code: RAISED_STATUS_CODE,
                blocked_content: false,
            }),
        }
    }
}

fn partial_texts(processed: &[String], texts: &[String], index: usize) -> Vec<String> {
    let mut partial = processed.to_vec();
    if let Some(remaining) = texts.get(index..) {
        partial.extend_from_slice(remaining);
    }
    partial
}
