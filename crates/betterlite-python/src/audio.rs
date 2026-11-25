mod convert;
mod error;
mod native;
mod session;
mod speech;
mod transcription;

use self::error::NativeAudioError;
use self::native::NativeAudioSession;
use pyo3::prelude::*;

pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<NativeAudioSession>()?;
    let error = module.py().get_type::<NativeAudioError>();
    module.add("NativeAudioError", error)
}
