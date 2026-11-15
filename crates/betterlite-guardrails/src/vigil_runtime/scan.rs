use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use indexmap::IndexMap;
use secrecy::ExposeSecret as _;

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
use super::error::VigilRuntimeError;

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
    let texts = bridged.texts.as_deref().unwrap_or_default();
    let scanned = match input_type {
        GuardrailInputType::Response => tool_call_arguments(bridged.tool_calls.as_deref()),
        GuardrailInputType::Request => Vec::new(),
    };
    if texts.iter().all(|text| is_blank(text)) && scanned.is_empty() {
        return Ok(inputs);
    }
    // httpx encodes header values as ASCII, so the failed encoding escapes apply_guardrail's except tuple
    if !guardrail.settings.api_key.expose_secret().is_ascii() {
        return Err(GuardrailFailure::from(VigilRuntimeError::NonAsciiApiKey));
    }
    let source = match input_type {
        GuardrailInputType::Request => Source::UserInput,
        GuardrailInputType::Response => Source::ModelOutput,
    };
    let metadata = logging_obj.map_or_else(
        || collect_metadata(request_data, None),
        |target| collect_metadata(request_data, Some(&CallIds { target })),
    );
    let run = Run {
        guardrail,
        original: &inputs,
        bridged: &bridged,
        source,
        metadata,
    };
    run.texts_then_tools(texts, &scanned).await
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

// A text-stage stop carries the partial result its fail_open answer needs
enum TextStop {
    Blocked(GuardrailFailure),
    Interrupted {
        error: VigilGuardError,
        partial: Vec<String>,
    },
    Unrecognized {
        partial: Vec<String>,
    },
}

// Every analyze failure is either a backend failure the fallback policy handles or a local error it must not swallow
enum FailureKind {
    Backend,
    Local,
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
            Some(Decision::Blocked) => Err(ScanStop::Blocked(self.raised(
                build_block_reason(&analysis),
                RAISED_STATUS_CODE,
                true,
            ))),
            None => Err(ScanStop::Unrecognized),
        }
    }

    async fn texts_then_tools(
        &self,
        texts: &[String],
        scanned: &[(usize, String)],
    ) -> Result<GuardrailApiInputs, GuardrailFailure> {
        match self.texts(texts).await {
            Ok(rewritten) => self.tool_calls(&rewritten, scanned).await,
            Err(TextStop::Blocked(failure)) => Err(failure),
            Err(TextStop::Interrupted { error, partial }) => self.interrupted(
                error,
                Partial {
                    texts: &partial,
                    tool_calls: self.bridged.tool_calls.as_deref(),
                },
            ),
            Err(TextStop::Unrecognized { partial }) => self.unrecognized(Partial {
                texts: &partial,
                tool_calls: self.bridged.tool_calls.as_deref(),
            }),
        }
    }

    async fn texts(&self, texts: &[String]) -> Result<Vec<String>, TextStop> {
        let mut rewritten = Vec::with_capacity(texts.len());
        for (index, text) in texts.iter().enumerate() {
            if is_blank(text) {
                rewritten.push(text.clone());
                continue;
            }
            match self.scan(text).await {
                Ok(scanned) => rewritten.push(scanned),
                Err(ScanStop::Blocked(failure)) => return Err(TextStop::Blocked(failure)),
                Err(ScanStop::Failed(error)) => {
                    return Err(TextStop::Interrupted {
                        error,
                        partial: partial_texts(&rewritten, texts, index),
                    });
                }
                Err(ScanStop::Unrecognized) => {
                    return Err(TextStop::Unrecognized {
                        partial: partial_texts(&rewritten, texts, index),
                    });
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
                    return self.interrupted(
                        error,
                        Partial {
                            texts,
                            tool_calls: rewritten.as_deref(),
                        },
                    );
                }
                Err(ScanStop::Unrecognized) => {
                    return self.unrecognized(Partial {
                        texts,
                        tool_calls: rewritten.as_deref(),
                    });
                }
            }
        }
        let built = build_output(self.bridged, texts, rewritten.as_deref());
        Ok(bridge::to_output(self.original, self.bridged, built))
    }

    fn raised(&self, message: String, status_code: u16, blocked_content: bool) -> GuardrailFailure {
        GuardrailFailure::GuardrailRaised {
            guardrail_name: self.guardrail.base.settings.guardrail_name.clone(),
            message,
            wrap_default_message: false,
            status_code,
            blocked_content,
        }
    }

    fn interrupted(
        &self,
        error: VigilGuardError,
        partial: Partial<'_>,
    ) -> Result<GuardrailApiInputs, GuardrailFailure> {
        // h11 raises a LocalProtocolError, an httpx.HTTPError, for a control character in the header, so Python treats it as a backend failure
        match failure_kind(&error) {
            FailureKind::Backend => {}
            FailureKind::Local => return Err(GuardrailFailure::from(error)),
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
        match handle_backend_failure(
            &context,
            &error,
            self.bridged,
            partial.texts,
            partial.tool_calls,
        ) {
            Ok(built) => Ok(bridge::to_output(self.original, self.bridged, built)),
            Err(backend) => {
                Err(self.raised(backend.to_string(), backend.status_code().as_u16(), false))
            }
        }
    }

    fn unrecognized(&self, partial: Partial<'_>) -> Result<GuardrailApiInputs, GuardrailFailure> {
        let guardrail_name = self.guardrail.base.settings.guardrail_name.as_deref();
        tracing::error!(
            guardrail_name = ?guardrail_name,
            source = ?self.source,
            "Vigil Guard returned an unrecognized decision"
        );
        match self.guardrail.unreachable_fallback {
            UnreachableFallback::FailOpen => {
                let built = build_output(self.bridged, partial.texts, partial.tool_calls);
                Ok(bridge::to_output(self.original, self.bridged, built))
            }
            UnreachableFallback::FailClosed => Err(self.raised(
                UNRECOGNIZED_DECISION_MESSAGE.to_owned(),
                RAISED_STATUS_CODE,
                false,
            )),
        }
    }
}

fn failure_kind(error: &VigilGuardError) -> FailureKind {
    match error {
        VigilGuardError::InvalidHeader { .. }
        | VigilGuardError::Transport { .. }
        | VigilGuardError::ResponseJson { .. } => FailureKind::Backend,
        VigilGuardError::MissingConfig { .. }
        | VigilGuardError::ToolCallIndex { .. }
        | VigilGuardError::InvalidToolFunction { .. }
        | VigilGuardError::Blocked { .. }
        | VigilGuardError::BackendUnavailable => FailureKind::Local,
    }
}

#[derive(Clone, Copy)]
struct Partial<'partial> {
    texts: &'partial [String],
    tool_calls: Option<&'partial [ToolCall]>,
}

// Python's str.strip also strips the four separators U+001C to U+001F
fn is_blank(text: &str) -> bool {
    for character in text.chars() {
        if !(character.is_whitespace() || matches!(character, '\u{1c}'..='\u{1f}')) {
            return false;
        }
    }
    true
}

fn partial_texts(processed: &[String], texts: &[String], index: usize) -> Vec<String> {
    let mut partial = processed.to_vec();
    if let Some(remaining) = texts.get(index..) {
        partial.extend_from_slice(remaining);
    }
    partial
}
