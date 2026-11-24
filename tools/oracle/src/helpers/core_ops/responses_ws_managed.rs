mod support;

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

use self::support::{Captured, QuotaRow, ScriptRow, ScriptedProvider, ScriptedSource, SinkRow};

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
        let shared = support::recorder();
        let sink = support::socket(&shared, &input.sink);
        let provider = ScriptedProvider::new(&shared, &input.scripts);
        let source = ScriptedSource::new(&shared, &input.frames, &input.disconnect);
        let mut handler = support::build_handler(&input.setup, sink, &shared, input.quota.as_ref());
        let captured = Captured::default();
        let subscriber = registry().with(captured.clone());
        runtime.block_on(
            answer(&shared, &captured, &mut handler, &provider, source).with_subscriber(subscriber),
        )
    }
}

async fn answer<Sink, Provider, Source>(
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
    Ok(json!({
        "frames": frame_rows,
        "provider_calls": support::calls(shared),
        "quota_calls": support::quotas(shared),
        "history": support::history(handler),
        "trace": support::trace(shared),
        "diagnostics": captured.take(),
    }))
}
