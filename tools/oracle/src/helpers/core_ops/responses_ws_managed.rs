mod fakes;
mod handler;
mod support;

use std::mem::take;

use betterlite_core::api::responses_ws_history::{FrameSink, ManagedResponsesWebSocket};
use betterlite_core::api::responses_ws_managed::provider::ResponsesStreaming;
use betterlite_core::api::responses_ws_managed::run::{FrameSource, run};
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::json;
use tokio::runtime::Builder;
use tracing::instrument::WithSubscriber as _;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::registry;

use self::fakes::ScriptedProvider;
use self::handler::{Captured, build_handler};
use self::support::{QuotaRow, ScriptRow, ScriptedSource, Shared, SinkRow, lock};

#[derive(Deserialize)]
struct Setup {
    model: String,
    litellm_metadata: Option<IndexMap<String, Json>>,
    custom_llm_provider: Option<String>,
    api_key: Option<String>,
    api_base: Option<String>,
    timeout: Option<f64>,
    first_message: Option<String>,
    kwargs: IndexMap<String, Json>,
}

#[derive(Deserialize)]
pub(crate) struct Input {
    setup: Setup,
    frames: Vec<String>,
    disconnect: String,
    scripts: Vec<ScriptRow>,
    quota: Option<QuotaRow>,
    sink: SinkRow,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread()
            .build()
            .map_err(|source| support::helper_error("RuntimeError", source.to_string()))?;
        let shared = Shared::default();
        let sink = support::socket(&shared, &input.sink);
        let provider = ScriptedProvider::new(&shared, &input.scripts);
        let source = ScriptedSource::new(&shared, &input.frames, &input.disconnect);
        let mut handler = build_handler(&input.setup, sink, &shared, input.quota.as_ref());
        let captured = Captured::default();
        let subscriber = registry().with(captured.clone());
        runtime.block_on(
            transcript(&shared, &captured, &mut handler, &provider, source)
                .with_subscriber(subscriber),
        )
    }
}

async fn transcript<Sink, Provider, Source>(
    shared: &support::Shared,
    captured: &Captured,
    handler: &mut ManagedResponsesWebSocket<Sink>,
    provider: &Provider,
    source: Source,
) -> HelperResult
where
    Sink: FrameSink,
    Provider: ResponsesStreaming,
    Source: FrameSource,
{
    run(handler, provider, source).await;
    let frame_rows = support::frames(shared)?;
    let (provider_calls, quota_calls) = support::recorded(shared);
    Ok(json!({
        "frames": frame_rows,
        "provider_calls": provider_calls,
        "quota_calls": quota_calls,
        "history": support::history(handler),
        "trace": json!(lock(shared).trace),
        "diagnostics": json!(take(&mut *lock(&captured.0))),
    }))
}
