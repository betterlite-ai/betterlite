use std::future::ready;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_config::sections::logging_privacy::Settings;
use betterlite_interactions::polling::host::{SettlementFuture, SettlementHost};
use betterlite_interactions::types::response::InteractionsApiResponse;
use betterlite_models::call_helpers::metadata::MetadataMap;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Serialize;
use thiserror::Error;

use super::dedupe::reset_success_emission_dedupe;
use super::reservation::{ReservationRelease, reservation_from_params};
use crate::exporter_options::failure_counter::CallbackFailureCounter;
use crate::exporter_options::redaction::ResponseStamp;
use crate::logging_state::LoggingState;
use crate::logging_state::correlation::CorrelationContext;
use crate::logging_success::handler::{SuccessHandler, async_success_handler};
use crate::logging_success::ports::{StandardPayloadBuilder, StreamAssembler, SuccessClock};
use crate::logging_success::types::{SuccessRequest, SuccessResult};
use crate::registry::CallbackRegistry;

const HIDDEN_PARAMS_KEY: &str = "_hidden_params";
const POLL_SCOPED_HIDDEN_PARAMS: [&str; 3] = ["response_cost", "model_id", "litellm_model_name"];

#[derive(Debug, Error)]
pub enum BillingError {
    #[error("the settled interaction response could not be converted to JSON: {source}")]
    ResponseJson {
        #[source]
        source: serde_json::Error,
    },
    #[error("the background interaction logging state is held by another settlement")]
    StateTaken,
}

pub struct InteractionLoggingState {
    pub logging: LoggingState,
    pub correlation: CorrelationContext,
}

// everything B250's SuccessHandler borrows, owned so the completion outlives the create's request
#[derive(Clone)]
pub struct InteractionLoggingCallbacks {
    pub registry: Arc<CallbackRegistry>,
    pub payload_builder: Arc<dyn StandardPayloadBuilder>,
    pub stream_assembler: Arc<dyn StreamAssembler>,
    pub clock: Arc<dyn SuccessClock>,
    pub failure_counters: Vec<Arc<dyn CallbackFailureCounter + Send + Sync>>,
    pub excluded_fields: Vec<String>,
    pub known_callback: Arc<dyn Fn(&str) -> bool + Send + Sync>,
    pub stamp: ResponseStamp,
    pub privacy: Settings,
}

// the poll registry clones its host into the context, so every clone settles the same logging state
#[derive(Clone)]
pub struct InteractionLoggingObject<Release> {
    pub state: Arc<Mutex<Option<InteractionLoggingState>>>,
    pub callbacks: InteractionLoggingCallbacks,
    pub release: Release,
}

impl<Release> SettlementHost for InteractionLoggingObject<Release>
where
    Release: ReservationRelease + Send + Sync,
{
    type Reservation = MetadataMap;
    type BillingError = BillingError;
    type ReleaseError = Release::Error;

    fn budget_reservation(&self) -> Option<MetadataMap> {
        let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let details = &state.as_ref()?.logging.model_call_details;
        let Some(Json::Object(params)) = details.get("litellm_params") else {
            return None;
        };
        reservation_from_params(params)
    }

    fn log_background_completion<'host>(
        &'host self,
        response: &'host InteractionsApiResponse,
    ) -> SettlementFuture<'host, Result<(), BillingError>> {
        let settled = match settled_response(response) {
            Ok(settled) => settled,
            Err(error) => return Box::pin(ready(Err(error))),
        };
        let taken = self
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        Box::pin(async move {
            let Some(mut held) = taken else {
                return Err(BillingError::StateTaken);
            };
            reset_success_emission_dedupe(&mut held.logging);
            let request = SuccessRequest {
                result: SuccessResult::Other(settled),
                start_time: None,
                end_time: None,
                cache_hit: None,
                prefer_async_handlers: false,
                standard_logging_object: None,
                guardrail_snapshot: None,
            };
            let handler = SuccessHandler {
                registry: self.callbacks.registry.as_ref(),
                payload_builder: self.callbacks.payload_builder.as_ref(),
                stream_assembler: self.callbacks.stream_assembler.as_ref(),
                clock: self.callbacks.clock.as_ref(),
                failure_counters: &self.callbacks.failure_counters,
                excluded_fields: &self.callbacks.excluded_fields,
                known_callback: self.callbacks.known_callback.as_ref(),
                stamp: &self.callbacks.stamp,
                privacy: self.callbacks.privacy,
            };
            async_success_handler(&handler, &mut held.logging, &mut held.correlation, &request)
                .await;
            *self.state.lock().unwrap_or_else(PoisonError::into_inner) = Some(held);
            Ok(())
        })
    }

    fn release_reservation<'host>(
        &'host self,
        reservation: &'host MetadataMap,
    ) -> SettlementFuture<'host, Result<(), Release::Error>> {
        self.release.release(reservation)
    }
}

fn settled_response(response: &InteractionsApiResponse) -> Result<Json, BillingError> {
    let mut hidden_params = response.hidden_params.clone();
    for key in POLL_SCOPED_HIDDEN_PARAMS {
        hidden_params.shift_remove(key);
    }
    let mut fields = object_from(response)?;
    fields.insert(HIDDEN_PARAMS_KEY.to_owned(), Json::Object(hidden_params));
    Ok(Json::Object(fields))
}

fn object_from<Value>(value: &Value) -> Result<IndexMap<String, Json>, BillingError>
where
    Value: Serialize,
{
    let text =
        serde_json::to_string(value).map_err(|source| BillingError::ResponseJson { source })?;
    serde_json::from_str(&text).map_err(|source| BillingError::ResponseJson { source })
}
