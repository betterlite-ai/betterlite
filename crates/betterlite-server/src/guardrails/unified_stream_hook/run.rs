use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::error::Error;
use std::mem;
use std::pin::Pin;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use betterlite_core::routes::catalog::RouteCatalog;
use betterlite_guardrails::guardrail_base::GuardrailBase;
use betterlite_guardrails::guardrail_base::contract::GuardrailContract;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::guardrail_base::inputs::GuardrailLogTarget;
use betterlite_guardrails::guardrail_message_scope::StreamItem;
use betterlite_guardrails::guardrail_selection::gate::GuardrailSelection as _;
use betterlite_guardrails::guardrail_translation::BaseTranslation;
use betterlite_guardrails::unified_dispatch::registry::{
    TranslationRegistry, resolve_endpoint_translation, resolve_transform_call_type,
};
use futures_util::StreamExt as _;
use futures_util::future::Either;
use futures_util::stream::{Stream, unfold};

use super::StreamHookParams;
use super::block_only::{HookOutput, Scanner, Window};
use super::end_scan::finish_stream;
use super::error::StreamHookError;
use super::settings::{StreamSettings, TransformMode};
use super::stream_end::StreamEndCheck;
use crate::guardrails::unified_stream::error::StreamLoopError;
use crate::guardrails::unified_stream::run::incremental_transform_stream;
use crate::guardrails::unified_stream::{StreamLoopParams, StreamOutput};

// What the block-only loop needs before the first chunk names the call type
struct BlockOnlyInputs<'run, T, G> {
    registry: &'run TranslationRegistry<T>,
    routes: &'run RouteCatalog,
    request_route: Option<&'run str>,
    guardrail: &'run G,
    guardrail_name: &'run str,
    request_data: &'run mut BTreeMap<String, GuardrailValue>,
    user_api_key_dict: &'run BTreeMap<String, GuardrailValue>,
    settings: StreamSettings,
}

enum Stage<'run, T, G> {
    Unresolved(BlockOnlyInputs<'run, T, G>),
    Scanning(Scanner<'run, T, G>),
    PassThrough,
}

enum Plan<'run, T, G> {
    Incremental(StreamLoopParams<'run, T, G>),
    Hook(Stage<'run, T, G>),
}

struct HookState<'run, T, G, S, E> {
    stage: Stage<'run, T, G>,
    source: Pin<Box<S>>,
    window: Window<E>,
    finished: bool,
}

// LiteLLM's UnifiedLLMGuardrails.async_post_call_streaming_iterator_hook for one guardrail
#[must_use = "the stream does nothing unless it is polled"]
pub fn run_streaming_iterator_hook<'run, T, G, S, E>(
    params: StreamHookParams<'run, T, G>,
    source: S,
) -> impl Stream<Item = Result<StreamOutput, StreamHookError<E>>> + Send + 'run
where
    T: BaseTranslation<Guardrail = G, LogTarget = GuardrailLogTarget, Failure = GuardrailFailure>
        + StreamEndCheck
        + Sync
        + 'run,
    G: GuardrailContract + 'run,
    S: Stream<Item = Result<StreamItem, E>> + Send + 'run,
    E: Error + Send + Sync + 'static,
{
    let state = match plan(params) {
        Ok(Plan::Incremental(loop_params)) => {
            let transformed = incremental_transform_stream(loop_params, source);
            return Either::Left(transformed.map(|result| result.map_err(StreamHookError::Loop)));
        }
        Ok(Plan::Hook(stage)) => HookState::new(Ok(stage), source),
        Err(error) => HookState::new(Err(error), source),
    };
    Either::Right(unfold(state, |mut current| async move {
        let output = current.next_output().await?;
        Some((output, current))
    }))
}

fn plan<T, G, E>(params: StreamHookParams<'_, T, G>) -> Result<Plan<'_, T, G>, StreamHookError<E>> {
    let Some(applied) = params.guardrail else {
        return Ok(Plan::Hook(Stage::PassThrough));
    };
    let name = applied
        .base
        .settings
        .guardrail_name
        .as_deref()
        .unwrap_or("None");
    // LiteLLM reads the flags, and logs the mask warning, before its gate; a bad flag fails only a stream it scans
    let resolved = StreamSettings::resolve(
        &params.flags,
        params.buffer_until_moderated_default,
        applied.base,
    );
    if !gate_open(applied.base, &*params.request_data)? {
        return Ok(Plan::Hook(Stage::PassThrough));
    }
    let inputs = BlockOnlyInputs {
        registry: params.registry,
        routes: params.routes,
        request_route: params.request_route,
        guardrail: applied.guardrail,
        guardrail_name: name,
        request_data: params.request_data,
        user_api_key_dict: params.user_api_key_dict,
        settings: resolved?,
    };
    if inputs.settings.transform_mode == TransformMode::IncrementalDiff {
        return Ok(transform_plan(inputs));
    }
    Ok(Plan::Hook(Stage::Unresolved(inputs)))
}

// LiteLLM runs incremental_diff only where the request route resolves to the chat-completions translation
fn transform_plan<T, G>(inputs: BlockOnlyInputs<'_, T, G>) -> Plan<'_, T, G> {
    let registry = inputs.registry;
    let transform = resolve_transform_call_type(registry, inputs.routes, inputs.request_route)
        .and_then(|call_type| registry.get(call_type));
    let Some(entry) = transform else {
        tracing::warn!(
            "UnifiedLLMGuardrails: streaming_transform_mode=incremental_diff is only supported for the OpenAI chat completions streaming path with a resolvable request route; falling back to block_only for {}",
            inputs.guardrail_name
        );
        return Plan::Hook(Stage::Unresolved(inputs));
    };
    Plan::Incremental(StreamLoopParams {
        translation: &entry.translation,
        guardrail: inputs.guardrail,
        request_data: inputs.request_data,
        user_api_key_dict: inputs.user_api_key_dict,
        call_type: entry.call_type,
        sampling_rate: inputs.settings.sampling_rate,
        end_of_stream_only: inputs.settings.end_of_stream_only,
    })
}

fn gate_open<E>(
    base: &GuardrailBase,
    request_data: &BTreeMap<String, GuardrailValue>,
) -> Result<bool, StreamHookError<E>> {
    let admitted = base
        .should_run_guardrail(
            request_data,
            GuardrailEventHooks::PostCall,
            &BTreeSet::new(),
        )
        .map_err(|source| StreamHookError::Gate { source })?;
    if !admitted {
        tracing::debug!(
            "UnifiedLLMGuardrails: Post-call streaming scanning disabled for {}",
            base.settings.guardrail_name.as_deref().unwrap_or("None")
        );
    }
    Ok(admitted)
}

impl<'run, T, G, S, E> HookState<'run, T, G, S, E>
where
    T: BaseTranslation<Guardrail = G, LogTarget = GuardrailLogTarget, Failure = GuardrailFailure>
        + StreamEndCheck
        + Sync,
    G: GuardrailContract,
    S: Stream<Item = Result<StreamItem, E>> + Send,
    E: Error + Send + Sync + 'static,
{
    // a plan error is the only output, so the source is never polled
    fn new(start: Result<Stage<'run, T, G>, StreamHookError<E>>, source: S) -> Self {
        let mut window = Window {
            chunk_counter: 0,
            responses_so_far: Vec::new(),
            responses_yielded: Vec::new(),
            withheld_items: Vec::new(),
            pending_items: Vec::new(),
            last_scan_key: None,
            tool_calls_in_flight: false,
            outputs: VecDeque::new(),
        };
        let (stage, finished) = match start {
            Ok(stage) => (stage, false),
            Err(error) => {
                window.outputs.push_back(Err(error));
                (Stage::PassThrough, true)
            }
        };
        Self {
            stage,
            source: Box::pin(source),
            window,
            finished,
        }
    }

    async fn next_output(&mut self) -> Option<HookOutput<E>> {
        loop {
            if let Some(output) = self.window.outputs.pop_front() {
                return Some(output);
            }
            if self.finished {
                return None;
            }
            match self.source.next().await {
                Some(Ok(item)) => self.consume(item).await,
                // LiteLLM lets an exception from the upstream iterator end the stream
                Some(Err(source)) => {
                    let failure = StreamLoopError::Source { source };
                    self.window
                        .outputs
                        .push_back(Err(StreamHookError::Loop(failure)));
                    self.finished = true;
                }
                None => {
                    self.finished = true;
                    self.end_scan().await;
                }
            }
        }
    }

    // LiteLLM scans everything received so far once the iterator ends; without a translation nothing is scanned
    async fn end_scan(&mut self) {
        if let Stage::Scanning(scanner) = &mut self.stage {
            finish_stream(&mut self.window, scanner).await;
        }
    }

    async fn consume(&mut self, item: StreamItem) {
        let stage = mem::replace(&mut self.stage, Stage::PassThrough);
        self.stage = match stage {
            Stage::Unresolved(inputs) => resolved_stage(inputs, &item),
            kept @ (Stage::Scanning(_) | Stage::PassThrough) => kept,
        };
        let Stage::Scanning(scanner) = &mut self.stage else {
            self.window.outputs.push_back(Ok(StreamOutput::Item(item)));
            return;
        };
        if self.window.scan_item(scanner, item).await.is_break() {
            self.finished = true;
        }
    }
}

// LiteLLM's call type is the route's first, else inferred from the first chunk; without a translation every chunk passes
fn resolved_stage<'run, T, G>(
    inputs: BlockOnlyInputs<'run, T, G>,
    first: &StreamItem,
) -> Stage<'run, T, G> {
    let BlockOnlyInputs {
        registry,
        routes,
        request_route,
        guardrail,
        guardrail_name,
        request_data,
        user_api_key_dict,
        settings,
    } = inputs;
    let resolved = resolve_endpoint_translation(registry, routes, request_route, Some(first));
    let Some((call_type, translation)) = resolved else {
        return Stage::PassThrough;
    };
    Stage::Scanning(Scanner {
        params: StreamLoopParams {
            translation,
            guardrail,
            request_data,
            user_api_key_dict,
            call_type,
            sampling_rate: settings.sampling_rate,
            end_of_stream_only: settings.end_of_stream_only,
        },
        buffering: settings.buffer_until_moderated,
        guardrail_name,
    })
}
