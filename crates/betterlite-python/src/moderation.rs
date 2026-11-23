mod convert;
mod error;
mod moderate;
mod native;
mod session;
mod text;

use self::error::NativeTextError;
use self::native::NativeTextSession;
use pyo3::prelude::*;

// main.py's text completion and moderation entry points: one session, its four call methods and the exception every failure carries.
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<NativeTextSession>()?;
    module.add("NativeTextError", module.py().get_type::<NativeTextError>())
}
