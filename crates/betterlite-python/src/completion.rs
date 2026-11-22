mod calls;
mod convert;
mod deployments;
mod error;
mod facts;
mod native;
mod ports;
mod session;
mod setup;
mod stream;
mod targets;
use self::error::NativeError;
use self::native::NativeSession;
use self::stream::NativeChatStream;
use pyo3::prelude::*;
// The classes litellm.main reaches for: one session, its streams and the exception every failure carries.
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<NativeSession>()?;
    module.add_class::<NativeChatStream>()?;
    module.add("NativeError", module.py().get_type::<NativeError>())?;
    Ok(())
}
