use std::sync::Arc;

use betterlite_a2a::card_resolver::CardResolver;
use betterlite_a2a::client::A2AClient;
use betterlite_a2a::client::create_a2a_client;
use pyo3::prelude::*;
use pyo3_async_runtimes::tokio::future_into_py;

use super::card;
use super::convert::{Call, Settings, parse};
use super::error::A2aCallError;
use super::send;
use super::setup::{Setup, card_headers, effective_client, resolve_timeout};
use super::stream;

#[pyclass(module = "betterlite._native")]
pub(crate) struct NativeA2aClient {
    #[pyo3(get)]
    base_url: String,
    client: A2AClient,
}

impl NativeA2aClient {
    // A2AClient is not Clone and a send takes it by value, so every call copies the handles it shares.
    fn copy_client(client: &A2AClient) -> A2AClient {
        A2AClient {
            http_client: client.http_client.clone(),
            call_context: client.call_context.clone(),
            agent_card: client.agent_card.clone(),
            streaming: client.streaming,
            endpoint: client.endpoint.clone(),
            timeout: client.timeout,
            tenant: client.tenant.clone(),
            request_ids: Arc::clone(&client.request_ids),
        }
    }
}

pub(crate) async fn create(settings: Settings) -> Result<NativeA2aClient, PyErr> {
    let setup = Setup::build()?;
    let base_url = settings.base_url.clone();
    let client = create_a2a_client(
        &setup.client,
        settings.into_settings(setup.tls.clone())?,
        &setup.environment,
    )
    .await
    .map_err(A2aCallError::from)?;
    Ok(NativeA2aClient { base_url, client })
}

// LiteLLM's aget_agent_card fetches the card and stops there, without normalizing its interfaces.
pub(crate) async fn fetch_card(settings: Settings) -> Result<String, PyErr> {
    let setup = Setup::build()?;
    let timeout = resolve_timeout(&setup.environment, settings.timeout)?;
    let client = effective_client(&setup.client, &setup.tls, timeout)?;
    let headers = card_headers(settings.extra_headers.as_ref())?;
    let resolver = CardResolver {
        base_url: settings.base_url,
        client,
    };
    let found = resolver
        .get_agent_card(settings.relative_card_path.as_deref(), &headers)
        .await
        .map_err(A2aCallError::from)?;
    Ok(card::text(&found)?)
}

#[pymethods]
impl NativeA2aClient {
    fn agent_card(&self) -> PyResult<String> {
        let mut found = self.client.agent_card.clone();
        card::fix(&mut found, &self.base_url);
        Ok(card::text(&found)?)
    }

    fn send_message<'py>(&self, py: Python<'py>, call: &str) -> PyResult<Bound<'py, PyAny>> {
        let wire: Call = parse(call)?;
        let client = Self::copy_client(&self.client);
        future_into_py(py, async move { send::send(Some(client), wire).await })
    }

    fn send_message_streaming<'py>(
        &self,
        py: Python<'py>,
        call: &str,
    ) -> PyResult<Bound<'py, PyAny>> {
        let wire: Call = parse(call)?;
        let client = Self::copy_client(&self.client);
        future_into_py(py, async move {
            let started = stream::start(Some(client), wire)?;
            Python::attach(|attached| Py::new(attached, started))
        })
    }
}
