use betterlite_core::api::fine_tuning_cancel_call::{
    FineTuningCancelDispatch, FineTuningCancelTarget,
};
use betterlite_core::api::fine_tuning_create::{FineTuningCreateDispatch, FineTuningCreateTarget};
use betterlite_core::api::fine_tuning_read::{FineTuningReadDispatch, ReadTarget};
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::outbound::OutboundRequest;
use betterlite_http::proxy::{ProxiedClient, UpstreamResponse};
use betterlite_models::decisions::Json;
use betterlite_openai::fine_tuning::job::{FineTuneJob, decode_fine_tune_job};

use super::convert::failures::provider_error;
use super::convert::requests::{WireCall, cancel_call, create_call, read_call, request_for};
use super::error::WireError;

// The client every native fine-tuning call sends through.
#[derive(Clone)]
pub(crate) struct WireDispatch {
    pub(crate) client: ProxiedClient,
}

impl WireDispatch {
    async fn send(&self, request: OutboundRequest) -> Result<UpstreamResponse, WireError> {
        let stream = self
            .client
            .send_following(request)
            .await
            .map_err(|source| WireError::Transport {
                source: Box::new(source),
            })?;
        stream
            .buffered(TransportLimits::DEFAULT.body_bytes)
            .await
            .map_err(|source| WireError::Transport {
                source: Box::new(source),
            })
    }

    // Sends the request one call builds and keeps the provider's answer when it succeeded.
    async fn exchange(
        &self,
        call: WireCall<'_>,
        integrations: Option<&[Json]>,
    ) -> Result<(UpstreamResponse, bool), WireError> {
        let (request, is_azure) = request_for(call, integrations)?;
        let response = accepted(self.send(request).await?)?;
        Ok((response, is_azure))
    }
}

fn accepted(response: UpstreamResponse) -> Result<UpstreamResponse, WireError> {
    if (200..300).contains(&response.status) {
        return Ok(response);
    }
    Err(provider_error(&response))
}

// The create call's loose integrations list rides beside the core record, which only takes objects.
#[derive(Clone)]
pub(crate) struct CreateDispatch {
    pub(crate) inner: WireDispatch,
    pub(crate) integrations: Option<Vec<Json>>,
}

impl FineTuningCreateDispatch for CreateDispatch {
    type Error = WireError;

    async fn create(&self, target: FineTuningCreateTarget) -> Result<FineTuneJob, Self::Error> {
        let (response, is_azure) = self
            .inner
            .exchange(create_call(&target)?, self.integrations.as_deref())
            .await?;
        decode_fine_tune_job(&response.body, is_azure)
            .map_err(|source| WireError::Decode { source })
    }
}

impl FineTuningReadDispatch for WireDispatch {
    type Error = WireError;

    async fn read(&self, target: ReadTarget<'_>) -> Result<Vec<u8>, Self::Error> {
        let (response, _is_azure) = self.exchange(read_call(&target), None).await?;
        Ok(response.body)
    }
}

impl FineTuningCancelDispatch for WireDispatch {
    type Error = WireError;

    async fn cancel(&self, target: FineTuningCancelTarget<'_>) -> Result<FineTuneJob, Self::Error> {
        let (response, is_azure) = self.exchange(cancel_call(&target), None).await?;
        decode_fine_tune_job(&response.body, is_azure)
            .map_err(|source| WireError::Decode { source })
    }
}
