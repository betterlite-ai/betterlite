use std::sync::Arc;

use betterlite_config::loader::startup::process_environment;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::sections::{batch_provider_list, xai_identity};
use betterlite_config::types::ProxyTrust;
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::batch_provider_cancel::service::BatchProviderCancel;
use betterlite_core::api::batch_provider_cancel::{BatchProviderCancelInput, CancelBatchCall};
use betterlite_core::api::batch_provider_list::{BatchProviderList, BatchProviderListInput};
use betterlite_core::api::resource_sdk_settings::ResourceSdkSettings;
use betterlite_http::proxy::ProxiedClient;
use betterlite_openai::api::batch_records::WireDecodeLimits;
use pyo3::prelude::*;
use pyo3_async_runtimes::tokio::{future_into_py, get_runtime};

use super::convert::{
    CallMode, CancelAnswer, HiddenParams, SessionWire, cancel_input, cancel_payload, list_input,
    list_payload, parse, section_settings,
};
use super::error::{ListingError, SessionError, listing_error};
use super::transport::{CancelDispatch, ListTransport};

// One Python batch listing session owns its environment snapshot, the section settings resolved once, and the pooled client.
pub(crate) struct ListingCore {
    client: ProxiedClient,
    sdk_profile: batch_provider_list::Settings,
    xai_profile: xai_identity::Settings,
    limits: ApiLimits,
    decode_limits: WireDecodeLimits,
}

impl ListingCore {
    pub(crate) fn build(text: &str) -> Result<Self, SessionError> {
        let wire: SessionWire = parse(text)?;
        let environment = wire.environment.unwrap_or_else(process_environment);
        let policy = ProxyPolicy::resolve(&ProxyTrust::default(), &environment)
            .map_err(|source| SessionError::Proxy { source })?;
        let client = ProxiedClient::outbound(&Arc::new(policy))
            .map_err(|source| SessionError::Client { source })?;
        let module = wire.settings;
        let limits = ApiLimits::default();
        Ok(Self {
            sdk_profile: section_settings::<batch_provider_list::Raw>(&module, &environment)?,
            xai_profile: section_settings::<xai_identity::Raw>(&module, &environment)?,
            decode_limits: WireDecodeLimits {
                max_json_bytes: limits.max_json_bytes,
                max_event_bytes: limits.max_event_bytes,
                max_items: limits.max_items,
            },
            limits,
            client,
        })
    }

    fn transport(&self) -> ListTransport {
        ListTransport {
            client: self.client.clone(),
            xai: self.xai_profile.clone(),
            limits: self.decode_limits,
        }
    }

    fn dispatch(&self) -> CancelDispatch {
        CancelDispatch {
            client: self.client.clone(),
            xai: self.xai_profile.clone(),
            limits: self.decode_limits,
        }
    }

    pub(crate) async fn list_with(
        &self,
        input: BatchProviderListInput,
        mode: CallMode,
    ) -> Result<String, ListingError> {
        let prepared = list_input(input, mode);
        let service = BatchProviderList {
            settings: ResourceSdkSettings::from(self.sdk_profile.clone()),
        };
        let result = service.run(&prepared)?;
        let answer = self.transport().list(&result.call, result.is_async).await?;
        serde_json::to_string(&answer).map_err(|source| ListingError::Encode { source })
    }

    pub(crate) async fn cancel_with(
        &self,
        call: CancelBatchCall,
        mode: CallMode,
    ) -> Result<String, ListingError> {
        let prepared = cancel_input(call, mode);
        let provider = prepared.provider.0.clone();
        let given_api_base = prepared.params.api_base.clone();
        // the clock is never read: an async invoke payload is refused before the provider service
        let service = BatchProviderCancel {
            clock: Arc::new(|| 0_i64),
            dispatch: self.dispatch(),
            limits: self.limits,
            settings: self.sdk_profile.clone(),
        };
        let input = BatchProviderCancelInput::Cancel(Box::new(prepared));
        let result = service.run(&input).await?;
        let answer = CancelAnswer {
            batch: &result.batch,
            hidden_params: HiddenParams::new(
                &provider,
                result.model.as_deref(),
                given_api_base.as_deref(),
            ),
        };
        serde_json::to_string(&answer).map_err(|source| ListingError::Encode { source })
    }
}

// the text a list or cancel call answered with, and the provider and model its failure names
async fn list_answer(
    core: Arc<ListingCore>,
    input: BatchProviderListInput,
    mode: CallMode,
) -> PyResult<String> {
    let provider = input.provider.name().to_owned();
    match core.list_with(input, mode).await {
        Ok(text) => Ok(text),
        Err(error) => Err(listing_error(&error, &provider, None)),
    }
}

async fn cancel_answer(
    core: Arc<ListingCore>,
    call: CancelBatchCall,
    mode: CallMode,
) -> PyResult<String> {
    let provider = call.provider.0.clone();
    let model = call.model.clone();
    core.cancel_with(call, mode)
        .await
        .map_err(|error| listing_error(&error, &provider, model.as_deref()))
}

// the file contents a batch payload carries, one blob per file it named
type Blobs = Vec<Vec<u8>>;

// main.py's list_batches, cancel_batch and the awaitable twins, one session owning the client.
#[pyclass(module = "betterlite._native")]
pub(crate) struct NativeBatchListSession {
    core: Arc<ListingCore>,
}

#[pymethods]
impl NativeBatchListSession {
    #[new]
    fn new(settings: &str) -> PyResult<Self> {
        let core = ListingCore::build(settings)?;
        Ok(Self {
            core: Arc::new(core),
        })
    }

    // the synchronous list: the provider call is already awaitable, so the runtime drives it here
    fn list(&self, py: Python<'_>, payload: &str, _blobs: Blobs) -> PyResult<String> {
        let input: BatchProviderListInput = list_payload(payload)?;
        let core = Arc::clone(&self.core);
        py.detach(move || get_runtime().block_on(list_answer(core, input, CallMode::Sync)))
    }

    // the synchronous cancel: cancel_batch's provider branches await here
    fn cancel(&self, py: Python<'_>, payload: &str, _blobs: Blobs) -> PyResult<String> {
        let call: CancelBatchCall = cancel_payload(payload)?;
        let core = Arc::clone(&self.core);
        py.detach(move || get_runtime().block_on(cancel_answer(core, call, CallMode::Sync)))
    }

    // the awaitable list: list_batches answers a coroutine when alist_batches is set
    fn alist(&self, py: Python<'_>, payload: &str, _blobs: Blobs) -> PyResult<Py<PyAny>> {
        let input: BatchProviderListInput = list_payload(payload)?;
        let core = Arc::clone(&self.core);
        future_into_py(py, list_answer(core, input, CallMode::Async)).map(Bound::unbind)
    }

    // the awaitable cancel: acancel_batch's provider branches await here
    fn acancel(&self, py: Python<'_>, payload: &str, _blobs: Blobs) -> PyResult<Py<PyAny>> {
        let call: CancelBatchCall = cancel_payload(payload)?;
        let core = Arc::clone(&self.core);
        future_into_py(py, cancel_answer(core, call, CallMode::Async)).map(Bound::unbind)
    }
}
