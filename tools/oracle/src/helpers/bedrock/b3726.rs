use std::time::Duration;

use betterlite_bedrock::guardrail_hooks::RequestData;
use betterlite_bedrock::guardrail_hooks::chunks::StreamChunk;
use betterlite_bedrock::guardrail_hooks::input::{InputHook, scan_input};
use betterlite_bedrock::guardrail_hooks::post_call::post_call_success_hook;
use betterlite_bedrock::guardrail_hooks::streaming::{StreamHook, streaming_iterator_hook};
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;
use serde_json::json;
use tokio::runtime::Builder;

mod assembly;
mod replies;
mod runtime;
mod support;
mod tests;
mod values;
mod view;

use self::replies::{Reply, Sent};
use self::runtime::OracleRuntime;
use self::support::{Routes, base_settings, build_guardrail, streaming_settings};

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum HookKind {
    PreCall,
    Moderation,
    PostCall,
    Streaming,
}

#[derive(Debug, Deserialize)]
struct GuardrailArgs {
    name: String,
    identifier: String,
    version: String,
    #[serde(default)]
    disable_exception_on_block: Option<bool>,
    #[serde(default)]
    latest_role_message_only: Option<bool>,
    #[serde(default)]
    event_hook: Option<GuardrailEventHooks>,
    #[serde(default)]
    buffer_until_moderated: Option<bool>,
    #[serde(default)]
    streaming_end_of_stream_only: Option<bool>,
    #[serde(default)]
    buffer_release_on_scan: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Args {
    hook: HookKind,
    call_type: String,
    guardrail: GuardrailArgs,
    data: RequestData,
    #[serde(default)]
    response: Option<GuardrailValue>,
    #[serde(default)]
    chunks: Vec<GuardrailValue>,
    #[serde(default)]
    sse_chunks: Vec<String>,
    #[serde(default)]
    request_route: Option<String>,
    #[serde(default)]
    keepalive_interval_seconds: Option<f64>,
    #[serde(default)]
    elapsed_seconds: Option<f64>,
    #[serde(default)]
    replies: Vec<Reply>,
}

fn stream_chunks(input: &Args) -> Vec<StreamChunk> {
    if input.sse_chunks.is_empty() {
        return input
            .chunks
            .iter()
            .cloned()
            .map(StreamChunk::Json)
            .collect();
    }
    input
        .sse_chunks
        .iter()
        .map(|frame| StreamChunk::Bytes(frame.clone().into_bytes()))
        .collect()
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Args;

    fn run(input: &Self::Input) -> HelperResult {
        let guardrail = build_guardrail(&input.guardrail)?;
        let base = base_settings(&input.guardrail);
        let settings = streaming_settings(
            &input.guardrail,
            &guardrail,
            input.keepalive_interval_seconds,
        );
        let elapsed = input
            .elapsed_seconds
            .and_then(|seconds| Duration::try_from_secs_f64(seconds).ok())
            .unwrap_or(Duration::ZERO);
        let runtime = OracleRuntime::new(
            guardrail,
            base,
            settings,
            Sent::new(input.replies.clone()),
            elapsed,
        );
        let mut data = input.data.clone();
        let mut response = input.response.clone().unwrap_or(GuardrailValue::Null);
        let mut chunks: Vec<StreamChunk> = Vec::new();
        let handle = Builder::new_current_thread()
            .enable_all()
            .build()
            .or_value_error()?;
        let outcome = handle.block_on(async {
            match input.hook {
                HookKind::PreCall => {
                    scan_input(&runtime, &mut data, &input.call_type, InputHook::PreCall).await
                }
                HookKind::Moderation => {
                    scan_input(&runtime, &mut data, &input.call_type, InputHook::Moderation).await
                }
                HookKind::PostCall => {
                    post_call_success_hook(&runtime, &mut data, &mut response).await
                }
                HookKind::Streaming => {
                    let routes = Routes::new();
                    let hook = StreamHook {
                        runtime: &runtime,
                        settings: &runtime.settings,
                        routes: &routes,
                        request_route: input.request_route.as_deref(),
                        data: &mut data,
                        chunks: stream_chunks(input),
                    };
                    match streaming_iterator_hook(hook).await {
                        Ok(items) => {
                            chunks = items;
                            Ok(())
                        }
                        Err(failure) => Err(failure),
                    }
                }
            }
        });
        Ok(json!({
            "outcome": view::outcome(outcome),
            "chunks": view::chunks(&chunks),
            "scans": runtime.transport.bodies().len(),
        }))
    }
}
