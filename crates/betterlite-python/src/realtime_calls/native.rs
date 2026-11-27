use std::sync::Arc;

use betterlite_http::proxy::UpstreamResponse;
use pyo3::prelude::*;
use pyo3_async_runtimes::tokio::future_into_py;

use super::convert;
use super::error::{CallFailure, SessionError};
use super::http;
use super::session::RealtimeSession;

type Blobs = Vec<Vec<u8>>;
type Pending = PyResult<Py<PyAny>>;

#[pyclass(module = "betterlite._native")]
pub(crate) struct NativeRealtimeSession {
    session: Arc<RealtimeSession>,
}

#[pymethods]
impl NativeRealtimeSession {
    #[new]
    fn new(settings: &str) -> PyResult<Self> {
        let session = RealtimeSession::build(settings).map_err(session_failure)?;
        Ok(Self {
            session: Arc::new(session),
        })
    }

    // acreate_realtime_client_secret: the provider's JSON body as text, or the class LiteLLM raises
    fn aclient_secret(&self, py: Python<'_>, payload: &str, _blobs: Blobs) -> Pending {
        let arguments = match convert::parse(payload) {
            Ok(arguments) => convert::client_secret(arguments),
            Err(error) => return Err(error.failure().python()),
        };
        let session = Arc::clone(&self.session);
        let future = async move {
            match http::client_secret(&session, &arguments).await {
                Ok(response) => body_text(response),
                Err(error) => Err(convert::http_failure(&error).python()),
            }
        };
        future_into_py(py, future).map(Bound::unbind)
    }

    // acreate_realtime_transcription_session: the provider's JSON body as text
    fn atranscription_session(&self, py: Python<'_>, payload: &str, _blobs: Blobs) -> Pending {
        let arguments = match convert::parse(payload) {
            Ok(arguments) => convert::transcription(arguments),
            Err(error) => return Err(error.failure().python()),
        };
        let session = Arc::clone(&self.session);
        let future = async move {
            match http::transcription(&session, &arguments).await {
                Ok(response) => body_text(response),
                Err(error) => Err(convert::http_failure(&error).python()),
            }
        };
        future_into_py(py, future).map(Bound::unbind)
    }

    // arealtime_calls: the SDP answer as text, with the offer arriving as the last blob
    fn acalls(&self, py: Python<'_>, payload: &str, blobs: Blobs) -> Pending {
        let arguments = calls_prepared(payload, &blobs);
        let session = Arc::clone(&self.session);
        let future = async move {
            match arguments {
                Ok(arguments) => match http::calls(&session, &arguments).await {
                    Ok(response) => body_text(response),
                    Err(error) => Err(convert::http_failure(&error).python()),
                },
                Err(error) => Err(error.python()),
            }
        };
        future_into_py(py, future).map(Bound::unbind)
    }
}

fn calls_prepared(
    payload: &str,
    blobs: &[Vec<u8>],
) -> Result<convert::CallsArguments, CallFailure> {
    let arguments = convert::parse(payload).map_err(|error| error.failure())?;
    convert::calls(arguments, blobs).map_err(|error| error.failure())
}

fn body_text(response: UpstreamResponse) -> PyResult<String> {
    String::from_utf8(response.body).map_err(|_error| {
        CallFailure::class(
            "ValueError",
            String::from("the realtime response body is not UTF-8"),
        )
        .python()
    })
}

fn session_failure(error: SessionError) -> PyErr {
    error.failure().python()
}
