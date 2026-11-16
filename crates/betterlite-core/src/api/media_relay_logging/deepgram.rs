use betterlite_logging::passthrough_logging_state::dispatch::PassThroughLoggingKwargs;
use betterlite_logging::passthrough_logging_state::response::PassThroughResponse;
use betterlite_logging::passthrough_stream_logging::types::HandlerOutcome;
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::decisions::Json;
use betterlite_models::duration_cost::openai_seconds::cost_per_second;
use betterlite_models::price_lookup::PriceLookup;
use indexmap::IndexMap;
use rust_decimal::Decimal;

use super::error::MediaRelayLoggingError;
use super::spend::{MediaSpend, decimal_json};
use crate::api::deepgram::listen_metadata::frames::{
    ListenFrame, audio_seconds, channel_count, transcript,
};
use crate::api::deepgram::listen_metadata::pricing::{
    PROVIDER, addon_pricing_models, is_priced, pricing_model,
};
use crate::api::deepgram::listen_metadata::target::{ListenTarget, parse_listen_target};

pub struct DeepgramListenRequest<'request> {
    pub upstream_url: &'request str,
    pub messages: &'request [Json],
    pub kwargs: &'request PassThroughLoggingKwargs,
    pub catalog: &'request ModelCatalog,
    pub lookup: &'request dyn PriceLookup,
}

pub struct DeepgramListenOutcome {
    pub outcome: HandlerOutcome,
    pub spend: MediaSpend,
    pub audio_seconds: Decimal,
    pub channels: u64,
    // The duration the registry bills for: the audio seconds counted once per channel.
    pub billed_seconds: Decimal,
    // Set exactly when the base rate is missing from the registry, the only case the source warns on.
    pub unpriced_registry_key: Option<String>,
}

pub fn deepgram_listen_outcome(
    request: &DeepgramListenRequest<'_>,
) -> Result<DeepgramListenOutcome, MediaRelayLoggingError> {
    let target = parse_listen_target(request.upstream_url)?;
    let frames = parsed_frames(request.messages);
    let seconds = audio_seconds(&frames);
    let channels = channel_count(&frames, target.query.declared_channels());
    let billed = seconds
        .checked_mul(Decimal::from(channels))
        .ok_or(MediaRelayLoggingError::Overflow)?;
    let (response_cost, unpriced_registry_key) = cost_for(&target, billed, request);
    let response = response_json(transcript(&frames), billed, response_cost);
    let spend = MediaSpend {
        model: target.query.requested_model().to_owned(),
        custom_llm_provider: PROVIDER,
        response_cost,
    };
    let outcome = HandlerOutcome {
        result: Some(PassThroughResponse::Transcription(response)),
        kwargs: spend.kwargs(request.kwargs),
    };
    Ok(DeepgramListenOutcome {
        outcome,
        spend,
        audio_seconds: seconds,
        channels,
        billed_seconds: billed,
        unpriced_registry_key,
    })
}

fn cost_for(
    target: &ListenTarget,
    billed: Decimal,
    request: &DeepgramListenRequest<'_>,
) -> (Option<Decimal>, Option<String>) {
    if is_priced(request.catalog, target) {
        return (audio_cost(billed, target, request.lookup), None);
    }
    let key = format!("{PROVIDER}/{}", pricing_model(target));
    (None, Some(key))
}

// The base must price for any cost to exist; an addon that fails to price contributes nothing.
fn audio_cost(billed: Decimal, target: &ListenTarget, lookup: &dyn PriceLookup) -> Option<Decimal> {
    let mut total = registry_cost(billed, &pricing_model(target), lookup)?;
    for addon in addon_pricing_models(target) {
        if let Some(cost) = registry_cost(billed, &addon, lookup) {
            total = total.checked_add(cost)?;
        }
    }
    Some(total)
}

// A lookup that fails logs debug and contributes nothing, the way the source's helpers swallow the error.
fn registry_cost(billed: Decimal, model: &str, lookup: &dyn PriceLookup) -> Option<Decimal> {
    let Ok((prompt, completion)) = cost_per_second(model, Some(PROVIDER), billed, lookup) else {
        tracing::debug!(model = %model, "deepgram listen passthrough has no registry price");
        return None;
    };
    prompt.checked_add(completion)
}

// A message that is not a listen frame is no candidate, the same as a mapping the source's reads miss.
fn parsed_frames(messages: &[Json]) -> Vec<ListenFrame> {
    messages
        .iter()
        .map(|message| {
            serde_json::to_value(message)
                .ok()
                .and_then(|value| serde_json::from_value::<ListenFrame>(value).ok())
                .unwrap_or(ListenFrame::Other)
        })
        .collect()
}

// The cost calculator reads the billed duration off the response, and the logger reads the cost there.
fn response_json(text: String, billed: Decimal, response_cost: Option<Decimal>) -> Json {
    let mut hidden = IndexMap::new();
    hidden.insert(
        "audio_transcription_duration".to_owned(),
        decimal_json(billed),
    );
    hidden.insert(
        "response_cost".to_owned(),
        response_cost.map_or(Json::Null, decimal_json),
    );
    let mut response = IndexMap::new();
    response.insert("text".to_owned(), Json::Text(text));
    response.insert("_hidden_params".to_owned(), Json::Object(hidden));
    Json::Object(response)
}
