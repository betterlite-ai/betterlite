mod convert;
mod error;
mod http;
mod native;
mod session;

use self::error::NativeRealtimeError;
use self::native::NativeRealtimeSession;
use pyo3::prelude::*;

// main.py's realtime entry points: one session, its async call methods and the exception every failure carries.
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<NativeRealtimeSession>()?;
    let error = module.py().get_type::<NativeRealtimeError>();
    module.add("NativeRealtimeError", error)
}
