mod answer;
mod convert;
mod error;
mod logging;
mod native;
mod provider;
mod session;
mod stream;
use self::error::NativeGenaiError;
use self::native::NativeGenaiSession;
use self::stream::NativeGenaiStream;
use pyo3::prelude::*;
// The classes betterlite.google_genai reaches for: one session, its streams and the exception every failure carries.
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<NativeGenaiSession>()?;
    module.add_class::<NativeGenaiStream>()?;
    module.add(
        "NativeGenaiError",
        module.py().get_type::<NativeGenaiError>(),
    )?;
    Ok(())
}
