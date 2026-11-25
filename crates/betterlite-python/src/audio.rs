mod convert;
mod error;
mod native;
mod session;
mod speech;
mod transcription;

use self::error::NativeAudioError;
use self::native::NativeAudioSession;
use pyo3::prelude::*;

// main.py's four audio entry points: one session, its four call methods and the exception every failure carries.
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<NativeAudioSession>()?;
    let error = module.py().get_type::<NativeAudioError>();
    module.add("NativeAudioError", error)
}
