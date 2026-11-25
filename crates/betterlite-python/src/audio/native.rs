use super::convert::{ConvertError, SpeechAnswer, TranscriptionAnswer, parse, parse_speech};
use super::error::{CallFailure, SessionError, class_error};
use super::session::AudioSession;
use super::speech;
use super::transcription;
use pyo3::prelude::*;
use pyo3_async_runtimes::tokio::{future_into_py, get_runtime};
use std::sync::Arc;

type Blobs = Vec<Vec<u8>>;
type Pending = PyResult<Py<PyAny>>;

#[pyclass(module = "betterlite._native")]
pub(crate) struct NativeAudioSession {
    session: Arc<AudioSession>,
}

#[pymethods]
impl NativeAudioSession {
    #[new]
    fn new(settings: &str) -> PyResult<Self> {
        Ok(Self {
            session: Arc::new(AudioSession::build(settings)?),
        })
    }

    // transcription: one attempt on the runtime, blocking this thread with the GIL released
    fn transcription(&self, py: Python<'_>, payload: &str, mut blobs: Blobs) -> PyResult<String> {
        let (call, files) =
            parse(payload, &mut blobs).map_err(|source| PyErr::from(SessionError::from(source)))?;
        let outcome =
            py.detach(|| get_runtime().block_on(transcription::run(&self.session, call, files)));
        encoded(outcome)
    }

    fn atranscription(&self, py: Python<'_>, payload: &str, mut blobs: Blobs) -> Pending {
        let (call, files) =
            parse(payload, &mut blobs).map_err(|source| PyErr::from(SessionError::from(source)))?;
        let session = Arc::clone(&self.session);
        let future = async move { encoded(transcription::run(&session, call, files).await) };
        future_into_py(py, future).map(Bound::unbind)
    }

    // speech: the audio comes back as bytes beside the status and headers, never as base64.
    fn speech(&self, py: Python<'_>, payload: &str, _blobs: Blobs) -> PyResult<(Vec<u8>, String)> {
        let call =
            parse_speech(payload).map_err(|source| PyErr::from(SessionError::from(source)))?;
        let outcome = py.detach(|| get_runtime().block_on(speech::run(&self.session, call)));
        answer(outcome)
    }

    fn aspeech(&self, py: Python<'_>, payload: &str, _blobs: Blobs) -> Pending {
        let call =
            parse_speech(payload).map_err(|source| PyErr::from(SessionError::from(source)))?;
        let session = Arc::clone(&self.session);
        let future = async move { answer(speech::run(&session, call).await) };
        future_into_py(py, future).map(Bound::unbind)
    }
}

fn encoded(outcome: Result<TranscriptionAnswer, CallFailure>) -> PyResult<String> {
    match outcome {
        Ok(answer) => serde_json::to_string(&answer)
            .map_err(|source| PyErr::from(SessionError::from(ConvertError::Encode { source }))),
        Err(failure) => Err(class_error(&failure)),
    }
}

fn answer(outcome: Result<SpeechAnswer, CallFailure>) -> PyResult<(Vec<u8>, String)> {
    match outcome {
        Ok(found) => {
            let text = serde_json::to_string(&found.header).map_err(|source| {
                PyErr::from(SessionError::from(ConvertError::Encode { source }))
            })?;
            Ok((found.content, text))
        }
        Err(failure) => Err(class_error(&failure)),
    }
}
