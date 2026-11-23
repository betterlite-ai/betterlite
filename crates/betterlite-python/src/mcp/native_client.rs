use std::future::ready;
use std::sync::{Arc, PoisonError};

use pyo3::prelude::*;
use pyo3_async_runtimes::tokio::future_into_py;

use super::client::ClientState;
use super::convert::{self, AuthValueWire, CallToolWire, ClientWire, GetPromptWire};
use super::native_session::NativeMcpSession;
use super::session::OpenSession;
use super::{Answer, awaited, setup};

// One native client per Python MCPClient: the settings text decides everything a session needs
#[pyclass(module = "betterlite._native")]
pub(crate) struct NativeMcpClient {
    state: Arc<ClientState>,
}

#[pymethods]
impl NativeMcpClient {
    #[new]
    fn new(py: Python<'_>, settings: &str) -> PyResult<Self> {
        let wire: ClientWire = convert::parse(settings)?;
        Ok(Self {
            state: Arc::new(setup::build(py, wire)?),
        })
    }

    #[pyo3(signature = (raise_on_error = false))]
    fn list_tools<'py>(&self, py: Python<'py>, raise_on_error: bool) -> Answer<'py> {
        let state = Arc::clone(&self.state);
        let work = async move { state.list_tools(raise_on_error).await };
        awaited(py, work)
    }

    fn call_tool<'py>(&self, py: Python<'py>, request: &str) -> Answer<'py> {
        let wire: CallToolWire = convert::parse(request)?;
        let state = Arc::clone(&self.state);
        let work = async move { state.call_tool(&wire).await };
        awaited(py, work)
    }

    #[pyo3(signature = (raise_on_error = false))]
    fn list_prompts<'py>(&self, py: Python<'py>, raise_on_error: bool) -> Answer<'py> {
        let state = Arc::clone(&self.state);
        let work = async move { state.list_prompts(raise_on_error).await };
        awaited(py, work)
    }

    fn get_prompt<'py>(&self, py: Python<'py>, request: &str) -> Answer<'py> {
        let wire: GetPromptWire = convert::parse(request)?;
        let state = Arc::clone(&self.state);
        let work = async move { state.get_prompt(wire).await };
        awaited(py, work)
    }

    #[pyo3(signature = (raise_on_error = false))]
    fn list_resources<'py>(&self, py: Python<'py>, raise_on_error: bool) -> Answer<'py> {
        let state = Arc::clone(&self.state);
        let work = async move { state.list_resources(raise_on_error).await };
        awaited(py, work)
    }

    #[pyo3(signature = (raise_on_error = false))]
    fn list_resource_templates<'py>(&self, py: Python<'py>, raise_on_error: bool) -> Answer<'py> {
        let state = Arc::clone(&self.state);
        let work = async move { state.list_resource_templates(raise_on_error).await };
        awaited(py, work)
    }

    fn read_resource<'py>(&self, py: Python<'py>, url: &str) -> Answer<'py> {
        let endpoint = url.to_owned();
        let state = Arc::clone(&self.state);
        let work = async move { state.read_resource(&endpoint).await };
        awaited(py, work)
    }

    fn discovery_auth_fingerprint<'py>(&self, py: Python<'py>) -> Answer<'py> {
        let state = Arc::clone(&self.state);
        let work = async move { state.discovery_auth_fingerprint() };
        awaited(py, work)
    }

    fn update_auth_value<'py>(&self, py: Python<'py>, value: &str) -> Answer<'py> {
        let wire: AuthValueWire = convert::parse(value)?;
        self.state.set_auth_value(convert::auth_value(wire));
        awaited(py, ready(Ok(())))
    }

    // the proxy reads the instructions initialize carried; B4744 strips them before storing
    fn instructions(&self) -> Option<String> {
        self.state
            .instructions
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    #[pyo3(signature = (quiet_on_error = false))]
    fn open_session<'py>(&self, py: Python<'py>, quiet_on_error: bool) -> Answer<'py> {
        let state = Arc::clone(&self.state);
        future_into_py(py, async move {
            let open = OpenSession::open(&state, quiet_on_error)
                .await
                .map_err(PyErr::from)?;
            Python::attach(|attached| {
                Py::new(
                    attached,
                    NativeMcpSession {
                        open: Arc::new(open),
                    },
                )
            })
        })
    }
}
