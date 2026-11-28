use std::sync::Arc;

use betterlite_http::proxy::UpstreamResponse;
use pyo3::prelude::*;
use pyo3_async_runtimes::tokio::future_into_py;

use super::convert;
use super::convert::ConvertError;
use super::error::CallFailure;
use super::http;
use super::http::Prepared;
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
        let session = RealtimeSession::build(settings)
            .map_err(|error| CallFailure::class("ValueError", error.to_string()).exception())?;
        Ok(Self {
            session: Arc::new(session),
        })
    }

    // acreate_realtime_client_secret: the provider's JSON body as text, or the class LiteLLM raises
    fn aclient_secret(&self, py: Python<'_>, payload: &str, _blobs: Blobs) -> Pending {
        let envelope: convert::Payload =
            serde_json::from_str(payload).map_err(ConvertError::Json)?;
        let arguments = convert::client_secret(envelope.arguments);
        answer(py, &self.session, Prepared::ClientSecret(arguments))
    }

    // acreate_realtime_transcription_session: the provider's JSON body as text
    fn atranscription_session(&self, py: Python<'_>, payload: &str, _blobs: Blobs) -> Pending {
        let envelope: convert::Payload =
            serde_json::from_str(payload).map_err(ConvertError::Json)?;
        let arguments = convert::transcription(envelope.arguments);
        answer(py, &self.session, Prepared::Transcription(arguments))
    }

    // arealtime_calls: the SDP answer as text, with the offer arriving as the last blob
    fn acalls(&self, py: Python<'_>, payload: &str, blobs: Blobs) -> Pending {
        let envelope: convert::Payload =
            serde_json::from_str(payload).map_err(ConvertError::Json)?;
        let arguments = convert::calls(envelope.arguments, blobs)?;
        answer(py, &self.session, Prepared::Calls(arguments))
    }
}

// The provider's answer as text, or the class LiteLLM raises for its failure
fn answer(py: Python<'_>, session: &Arc<RealtimeSession>, prepared: Prepared) -> Pending {
    let owned = Arc::clone(session);
    let future = async move {
        match http::exchange(&owned, &prepared).await {
            Ok(response) => body_text(response),
            Err(error) => Err(convert::http_failure(&error).exception()),
        }
    };
    future_into_py(py, future).map(Bound::unbind)
}

fn body_text(response: UpstreamResponse) -> PyResult<String> {
    String::from_utf8(response.body).map_err(|_error| {
        CallFailure::class(
            "ValueError",
            String::from("the realtime response body is not UTF-8"),
        )
        .exception()
    })
}

mod tests;
