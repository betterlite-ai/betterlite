use std::collections::BTreeMap;
use std::future::{Future, ready};

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_core::guardrails::streaming_rewrite::failure::UndeliverableStreamRewrite;
use betterlite_core::guardrails::streaming_rewrite::shapes::{RequestData, StreamInputs};
use betterlite_core::guardrails::streaming_rewrite::step::{StreamingStep, run_streaming_step};
use betterlite_core::guardrails::streaming_rewrite::traits::{
    ApplySurface, GuardrailCall, GuardrailCallback, HookSurface, InputType, NamedGuardrail,
    PostCallHook, PostCallSurface, RewriteFailure, StreamCall, StreamGuardrail, StreamTranslation,
    UserApiKey,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::helpers::{Helper, HelperError, HelperResult};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    guardrail: GuardrailScript,
    translation: TranslationScript,
    chunks: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GuardrailScript {
    kind: Kind,
    name: String,
    texts_out: Option<Vec<String>>,
    tool_calls_out: Option<Vec<GuardrailValue>>,
    hook: HookScript,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Kind {
    Unified,
    Legacy,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct HookScript {
    mode: HookMode,
    response: Option<Assembled>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum HookMode {
    Nothing,
    Replace,
    Mutate,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Assembled {
    texts: Vec<String>,
    tool_calls: Vec<GuardrailValue>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TranslationScript {
    delivers: bool,
    tool_calls: Option<Vec<GuardrailValue>>,
    rescans: bool,
}

#[derive(Debug, Error)]
enum StepError {
    #[error(transparent)]
    Undeliverable(#[from] UndeliverableStreamRewrite),
    #[error("a streaming step always passes request_data")]
    NoRequestData,
}

impl RewriteFailure for StepError {
    fn undeliverable(&self) -> Option<&UndeliverableStreamRewrite> {
        match self {
            Self::Undeliverable(undeliverable) => Some(undeliverable),
            Self::NoRequestData => None,
        }
    }
}

struct Scripted<'script> {
    script: &'script GuardrailScript,
}

impl NamedGuardrail for Scripted<'_> {
    fn guardrail_name(&self) -> Option<&str> {
        Some(&self.script.name)
    }
}

impl StreamGuardrail<()> for Scripted<'_> {
    type Failure = StepError;

    fn covers_full_request(&self) -> bool {
        false
    }

    fn apply_guardrail(
        &mut self,
        inputs: StreamInputs,
        _call: GuardrailCall<'_, ()>,
    ) -> impl Future<Output = Result<StreamInputs, StepError>> + Send {
        ready(Ok(StreamInputs {
            texts: self.script.texts_out.clone().or(inputs.texts),
            tool_calls: self.script.tool_calls_out.clone().or(inputs.tool_calls),
            extra: inputs.extra,
        }))
    }
}

impl GuardrailCallback<()> for Scripted<'_> {
    fn hook_surface(&self) -> HookSurface {
        let unified = matches!(self.script.kind, Kind::Unified);
        HookSurface {
            apply: ApplySurface {
                own_apply_guardrail: unified,
                use_native_lifecycle_hooks: false,
            },
            post_call: PostCallSurface {
                own_success_hook: !unified,
                own_streaming_iterator_hook: false,
            },
        }
    }

    fn records_own_information(&self) -> bool {
        false
    }

    fn async_post_call_success_hook(
        &mut self,
        call: PostCallHook<'_>,
    ) -> impl Future<Output = Result<Option<GuardrailValue>, StepError>> + Send {
        let response = self
            .script
            .hook
            .response
            .as_ref()
            .map(|scripted| assembled_value(&scripted.texts, &scripted.tool_calls));
        ready(Ok(match self.script.hook.mode {
            HookMode::Nothing => None,
            HookMode::Replace => response,
            HookMode::Mutate => {
                *call.response = response;
                None
            }
        }))
    }
}

fn assembled_value(texts: &[String], tool_calls: &[GuardrailValue]) -> GuardrailValue {
    let text_items = texts.iter().cloned().map(GuardrailValue::Text).collect();
    GuardrailValue::Object(BTreeMap::from([
        ("texts".to_owned(), GuardrailValue::Array(text_items)),
        (
            "tool_calls".to_owned(),
            GuardrailValue::Array(tool_calls.to_vec()),
        ),
    ]))
}

fn texts_of(items: &[GuardrailValue]) -> Vec<String> {
    let mut texts = Vec::with_capacity(items.len());
    for item in items {
        if let GuardrailValue::Text(text) = item {
            texts.push(text.clone());
        }
    }
    texts
}

fn scanned_inputs(response: &GuardrailValue) -> Option<StreamInputs> {
    let GuardrailValue::Object(fields) = response else {
        return None;
    };
    let Some(GuardrailValue::Array(texts)) = fields.get("texts") else {
        return None;
    };
    let Some(GuardrailValue::Array(tool_calls)) = fields.get("tool_calls") else {
        return None;
    };
    Some(StreamInputs {
        texts: Some(texts_of(texts)),
        tool_calls: Some(tool_calls.clone()),
        extra: BTreeMap::new(),
    })
}

struct Translation<'script, const DELIVERS: bool> {
    script: &'script TranslationScript,
}

impl<const DELIVERS: bool> StreamTranslation<()> for Translation<'_, DELIVERS> {
    type Failure = StepError;

    const DELIVERS_ENDED_STREAM_REWRITES: bool = DELIVERS;

    async fn process_output_response<Guardrail>(
        &self,
        response: GuardrailValue,
        guardrail: &mut Guardrail,
        call: StreamCall<'_, ()>,
    ) -> Result<(), StepError>
    where
        Guardrail: StreamGuardrail<(), Failure = StepError>,
    {
        let Some(inputs) = scanned_inputs(&response).filter(|_| self.script.rescans) else {
            return Ok(());
        };
        let mut request_data = RequestData::new();
        let call_args = GuardrailCall {
            request_data: &mut request_data,
            input_type: InputType::Response,
            logging_obj: call.logging_obj,
        };
        guardrail.apply_guardrail(inputs, call_args).await?;
        Ok(())
    }

    async fn process_streaming_response<Guardrail>(
        &self,
        responses_so_far: &mut Vec<GuardrailValue>,
        guardrail: &mut Guardrail,
        call: StreamCall<'_, ()>,
    ) -> Result<(), StepError>
    where
        Guardrail: StreamGuardrail<(), Failure = StepError>,
    {
        let request_data = call.request_data.ok_or(StepError::NoRequestData)?;
        let texts = texts_of(responses_so_far);
        let tool_calls = self.script.tool_calls.clone();
        let stored_tools = tool_calls.clone().unwrap_or_default();
        request_data.insert(
            "response".to_owned(),
            assembled_value(&texts, &stored_tools),
        );
        let inputs = StreamInputs {
            texts: Some(texts),
            tool_calls,
            extra: BTreeMap::new(),
        };
        let call_args = GuardrailCall {
            request_data,
            input_type: InputType::Response,
            logging_obj: call.logging_obj,
        };
        let outputs = guardrail.apply_guardrail(inputs, call_args).await?;
        if call.deliver_ended_stream_rewrites
            && let Some(rewritten) = outputs.texts
            && rewritten.len() == responses_so_far.len()
        {
            *responses_so_far = rewritten.into_iter().map(GuardrailValue::Text).collect();
        }
        if let Some(delivered) = outputs.tool_calls {
            request_data.insert(
                "delivered_tool_calls".to_owned(),
                GuardrailValue::Array(delivered),
            );
        }
        Ok(())
    }
}

fn record_applied(request_data: &mut RequestData, guardrail_name: &str) {
    let applied = request_data
        .entry("applied_guardrails".to_owned())
        .or_insert_with(|| GuardrailValue::Array(Vec::new()));
    if let GuardrailValue::Array(names) = applied {
        names.push(GuardrailValue::Text(guardrail_name.to_owned()));
    }
}

#[derive(Debug, Error)]
enum RunError {
    #[error(transparent)]
    Step(#[from] StepError),
    #[error(transparent)]
    Runtime(#[from] std::io::Error),
    #[error(transparent)]
    Encode(#[from] serde_json::Error),
}

// LiteLLM's executor turns a step exception into outcome "error", so only a passing step reaches the output
#[derive(Serialize)]
struct Output {
    outcome: &'static str,
    chunks: Vec<GuardrailValue>,
    applied_guardrails: GuardrailValue,
    delivered_tool_calls: Option<GuardrailValue>,
}

async fn drive<const DELIVERS: bool>(input: &Input) -> Result<Output, StepError> {
    let mut guardrail = Scripted {
        script: &input.guardrail,
    };
    let translation = Translation::<DELIVERS> {
        script: &input.translation,
    };
    let mut chunks: Vec<GuardrailValue> = input
        .chunks
        .iter()
        .cloned()
        .map(GuardrailValue::Text)
        .collect();
    let mut hook_input = RequestData::new();
    let user_api_key_dict = UserApiKey::new();
    let step = StreamingStep {
        guardrail: &input.guardrail.name,
        streaming_chunks: &mut chunks,
        hook_input: &mut hook_input,
        user_api_key_dict: &user_api_key_dict,
        logging_obj: None::<&mut ()>,
    };
    run_streaming_step(&mut guardrail, &translation, step, record_applied).await?;
    Ok(Output {
        outcome: "pass",
        chunks,
        applied_guardrails: hook_input
            .remove("applied_guardrails")
            .unwrap_or_else(|| GuardrailValue::Array(Vec::new())),
        delivered_tool_calls: hook_input.remove("delivered_tool_calls"),
    })
}

fn execute(input: &Input) -> Result<serde_json::Value, RunError> {
    let runtime = tokio::runtime::Builder::new_current_thread().build()?;
    let output = if input.translation.delivers {
        runtime.block_on(drive::<true>(input))?
    } else {
        runtime.block_on(drive::<false>(input))?
    };
    Ok(serde_json::to_value(output)?)
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        execute(input).map_err(|error| {
            Box::new(HelperError {
                class: "ValueError".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })
    }
}
