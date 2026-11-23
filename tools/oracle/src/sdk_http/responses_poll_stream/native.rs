use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::responses_poll_state::DEFAULT_TTL;
use betterlite_core::api::responses_poll_state::cache::PollingCache;
use betterlite_core::api::responses_poll_state::get_cache_key;
use betterlite_core::api::responses_poll_state::store::ResponsePollingStore;
use betterlite_core::api::responses_poll_stream::ports::PollRequest;
use betterlite_core::api::responses_poll_stream::worker::BackgroundStreaming;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};

use super::Call;
use super::Outcome;
use super::capture::CaptureCache;
use super::clock::ScriptedClock;
use super::process::ProviderProcessor;
use super::request::{Paused, scope};
use crate::sdk_http::error::SdkHttpError;
use crate::sdk_http::result::kinds::SdkResult;
use crate::sdk_http::vector_query_embedding::outbound_client;

// the production worker over the mock provider, with only the clock, the cache and admission scripted
pub(super) async fn execute(
    call: &Call,
    api_base: &str,
    env: &BTreeMap<String, String>,
) -> Result<SdkResult, SdkHttpError> {
    let key = get_cache_key(&call.polling_id);
    let seeded = CaptureCache::new(key, &call.seed, call.cancel_after_write)?;
    let capture = Arc::new(seeded);
    let cache: Arc<dyn PollingCache> = Arc::<CaptureCache>::clone(&capture);
    let store = ResponsePollingStore {
        cache: Some(cache),
        ttl: DEFAULT_TTL,
        limits: ApiLimits::default(),
    };
    let processor = ProviderProcessor::new(outbound_client(env)?, api_base)?;
    let clock = ScriptedClock::new(&call.clock_millis);
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(SdkHttpError::Redactor)?;
    let request = PollRequest {
        scope: scope(call)?,
        inbound: Box::new(Paused),
    };
    BackgroundStreaming {
        polling_id: &call.polling_id,
        data: call.data.clone(),
        request,
        store: &store,
        processor: &processor,
        clock: &clock,
        redactor: &redactor,
        limits: ApiLimits::default(),
    }
    .run()
    .await
    .map_err(|error| SdkHttpError::Invalid(format!("the poll stream worker failed: {error}")))?;
    Ok(SdkResult::ResponsesPollStream {
        value: Box::new(Outcome {
            writes: capture.writes(),
        }),
    })
}
